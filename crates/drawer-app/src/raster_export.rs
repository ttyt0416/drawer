use super::*;
use drawer_core::{
    color_precision::Depth,
    raster_io::{self as core, Compression, Format, Options},
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    source: Option<u64>,
    path: String,
    options: Options,
    confirmed: bool,
    replace: bool,
    prepared: Option<core::Prepared>,
    encoded: Option<core::Encoded>,
    jpeg_attempt: Option<Options>,
    job: Option<Job>,
    preview: Option<egui::TextureHandle>,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
enum Value {
    Source(core::Prepared),
    Encoded(core::Encoded),
}
struct Job {
    cancel: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    rx: mpsc::Receiver<Result<Value, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Drawer {
    pub(super) fn begin_raster_export(&mut self) -> Result<(), String> {
        self.finish_stroke();
        if self.drawing.has_path_draft() {
            return Err("Commit or cancel the path draft before exporting".into());
        }
        let n = self.active.ok_or("Open a document first")?;
        let d = &self.documents[n];
        self.raster_export = Panel::default();
        let available = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        copy.id = d.id;
        copy.revision = d.revision;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("prepare-still-image".into())
            .spawn(move || {
                let _ = tx.send(core::prepare(&copy, available, &worker).map(Value::Source));
            })
            .map_err(|e| e.to_string())?;
        self.raster_export = Panel {
            open: true,
            source: Some(d.id),
            job: Some(Job {
                cancel,
                rx,
                handle: Some(handle),
            }),
            message: "Preparing source composite…".into(),
            ..Default::default()
        };
        Ok(())
    }
    fn save_raster_export(&self, p: &Panel) -> Result<(), String> {
        let n = self.active.ok_or("Open the source document")?;
        if Some(self.documents[n].id) != p.source {
            return Err("Activate the original source document".into());
        }
        let prepared = p.prepared.as_ref().ok_or("Wait for the export preview")?;
        let target = self.export_target(n, &PathBuf::from(&p.path))?;
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        if p.options.format == Format::Jpeg {
            let encoded = p
                .encoded
                .as_ref()
                .filter(|v| v.options() == p.options.applied())
                .ok_or("Wait for the actual JPEG compression preview")?;
            prepared.write_encoded(
                &self.documents[n],
                &target,
                encoded,
                p.confirmed,
                p.replace,
                available,
                &AtomicBool::new(false),
            )
        } else {
            prepared.write(
                &self.documents[n],
                &target,
                p.options.applied(),
                p.confirmed,
                p.replace,
                available,
                &AtomicBool::new(false),
            )
        }
    }
    fn prepare_jpeg_preview(&self, p: &mut Panel) -> Result<(), String> {
        let prepared = p.prepared.as_ref().ok_or("Prepare the source first")?;
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(
                self.documents
                    .iter()
                    .map(Document::resident_bytes)
                    .sum::<usize>(),
            )
            .saturating_sub(prepared.pixels.len() * 8);
        if prepared.pixels.len().saturating_mul(48) > available {
            return Err("JPEG preview copy exceeds workspace memory".into());
        }
        let prepared = prepared.clone();
        let o = p.options.applied();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let (tx, rx) = mpsc::channel();
        p.jpeg_attempt = Some(o);
        let handle = std::thread::Builder::new()
            .name("jpeg-compression-preview".into())
            .spawn(move || {
                let _ = tx.send(
                    prepared
                        .encoded_preview(o, available, &worker)
                        .map(Value::Encoded),
                );
            })
            .map_err(|e| e.to_string())?;
        p.job = Some(Job {
            cancel,
            rx,
            handle: Some(handle),
        });
        p.jpeg_attempt = Some(o);
        Ok(())
    }

    pub(super) fn raster_export_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.raster_export);

        if let Some(job) = &p.job {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
            let result = match job.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("Still-image worker stopped".into())),
            };
            if let Some(result) = result {
                p.job = None;
                match result {
                    Ok(Value::Source(prepared)) => {
                        p.prepared = Some(prepared);
                        p.message = "Review format options and losses before exporting".into();
                    }
                    Ok(Value::Encoded(encoded)) => {
                        if encoded.options() == p.options.applied() {
                            p.preview = Some(ctx.load_texture(
                                "actual-jpeg-preview",
                                egui::ColorImage::from_rgba_unmultiplied(
                                    encoded.size,
                                    &encoded.pixels.iter().flatten().copied().collect::<Vec<_>>(),
                                ),
                                egui::TextureOptions::LINEAR,
                            ));
                            p.message = format!(
                                "Actual JPEG: {} bytes. These exact reviewed bytes will be written.",
                                encoded.bytes().len()
                            );
                            p.encoded = Some(encoded);
                        }
                    }
                    Err(e) => p.message = e,
                }
            }
        }
        if !p.open {
            self.raster_export = p;
            return;
        }
        let before = p.options;
        let mut open = true;
        let mut export = false;
        let mut cancel = false;
        let mut browse = false;
        let mut watermark = false;
        p.controls.clear();
        egui::Window::new("Export still image — format and losses")
            .default_pos(egui::pos2(25., 25.))
            .default_width(650.)
            .max_height(640.)
            .vscroll(true)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add_enabled(
                        p.prepared
                            .as_ref()
                            .is_some_and(|v| p.confirmed || v.losses(p.options.applied()).is_empty())
                            && !p.path.is_empty() && p.job.is_none()
                            && (p.options.format!=Format::Jpeg || p.encoded.as_ref().is_some_and(|v|v.options()==p.options.applied())),
                        egui::Button::new("Export image"),
                    );
                    p.controls.insert("export", r.rect);
                    export = r.clicked();
                    let r = ui.button(crate::i18n::text("Cancel"));
                    p.controls.insert("cancel", r.rect);
                    cancel = r.clicked();
                });
                if p.prepared
                    .as_ref()
                    .is_some_and(|v| !v.losses(p.options.applied()).is_empty())
                {
                    let r = ui.add_enabled(p.options.format!=Format::Jpeg||p.encoded.is_some(),egui::Checkbox::new(&mut p.confirmed,"I reviewed the losses in this exported copy"));
                    p.controls.insert("confirm", r.rect);
                }
                watermark=ui.button("Watermark, noise and output size…").clicked();
                ui.horizontal_wrapped(|ui| {
                    for format in Format::ALL {
                        let r = ui.selectable_value(&mut p.options.format, format, format.label());
                        p.controls.insert(format.extension(), r.rect);
                    }
                });
                let r=ui.checkbox(&mut p.options.grayscale,"Grayscale output (PNG Gray/GrayAlpha; others equal RGB)");p.controls.insert("grayscale",r.rect);
                if p.options.format == Format::Jpeg {
                    let r=ui.add(egui::Slider::new(&mut p.options.quality, 1..=100).text("JPEG luma quality"));p.controls.insert("jpeg-luma",r.rect);
                    let r=ui.add(egui::Slider::new(&mut p.options.chroma_quality,1..=100).text("JPEG chroma quality"));p.controls.insert("jpeg-chroma",r.rect);
                    ui.horizontal(|ui|{for(key,v)in[("444",drawer_core::jpeg_output::Sampling::S444),("422",drawer_core::jpeg_output::Sampling::S422),("420",drawer_core::jpeg_output::Sampling::S420)]{let r=ui.selectable_value(&mut p.options.sampling,v,v.label());p.controls.insert(key,r.rect);}});
                    if p.encoded.is_none() && p.job.is_none() && p.jpeg_attempt.is_some() && ui.button("Retry compression preview").clicked(){p.jpeg_attempt=None;}
                    let mut unsupported=false;ui.add_enabled(false,egui::Checkbox::new(&mut unsupported,"Progressive / arithmetic / custom tables"));ui.small("Unavailable in this adapter: only baseline sequential RGB8 and the reviewed quality-derived tables are supported.");
                } else {
                    let r = ui.checkbox(&mut p.options.alpha, if p.options.format==Format::Bmp{"BMP32 alpha (V5); off = BMP24 opaque (INFO)"}else{"Preserve alpha"});
                    p.controls.insert("alpha", r.rect);
                }
                if !p.options.applied().alpha {
                    ui.color_edit_button_srgb(&mut p.options.background);
                    ui.label("Opaque background RGB");
                }
                if matches!(p.options.format, Format::Tiff | Format::Png) {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut p.options.depth, Depth::Eight, "8-bit channels");
                        let r = ui.selectable_value(
                            &mut p.options.depth,
                            Depth::Sixteen,
                            "16-bit channels",
                        );
                        p.controls.insert("sixteen", r.rect);
                    });
                    egui::ComboBox::from_id_salt("still-compression")
                        .selected_text(format!("{:?}", p.options.applied().compression))
                        .show_ui(ui, |ui| {
                            for c in [
                                Compression::None,
                                Compression::Lzw,
                                Compression::Deflate,
                                Compression::Packbits,
                            ] {
                                if p.options.format == Format::Png
                                    && !matches!(c, Compression::None | Compression::Deflate)
                                {
                                    continue;
                                }
                                ui.selectable_value(
                                    &mut p.options.compression,
                                    c,
                                    format!("{c:?}"),
                                );
                            }
                        });
                }
                if p.options.format == Format::Tga {
                    ui.checkbox(&mut p.options.rle, "TGA RLE compression");
                }
                if let Some(prepared) = &p.prepared {
                    ui.label(format!(
                        "{} × {} pixels; source {:?}",
                        prepared.width, prepared.height, prepared.source_depth
                    ));
                    let losses = prepared.losses(p.options.applied());
                    if losses.is_empty() {
                        ui.small("No editing/pixel losses detected for these settings");
                    } else {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Losses in this exported copy; keep the editable source",
                        );
                        for loss in losses {
                            ui.label(loss);
                        }
                    }
                    ui.collapsing("Output details", |ui| {
                        for note in prepared.notices(p.options.applied()) {
                            ui.small(note);
                        }
                    });
                }
                if ui.text_edit_singleline(&mut p.path).changed() {
                    p.replace = false;
                }
                browse = ui.button(crate::i18n::text("Save as…")).clicked();
                ui.checkbox(&mut p.replace, "Replace this existing export file");
                ui.label(&p.message);
                if let Some(texture) = &p.preview {
                    let size = texture.size_vec2();
                    let scale = (300. / size.x).min(100. / size.y).min(1.);
                    ui.image((texture.id(), size * scale));
                }
            });
        if p.options != before {
            p.confirmed = false;
            p.preview = None;
            p.encoded = None;
            p.jpeg_attempt = None;
            if p.prepared.is_some()
                && let Some(job) = &p.job
            {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        if p.options.format == Format::Jpeg
            && p.encoded.is_none()
            && p.jpeg_attempt.is_none()
            && p.prepared.is_some()
            && p.job.is_none()
            && let Err(e) = self.prepare_jpeg_preview(&mut p)
        {
            p.message = e;
        }
        if p.options.format != Format::Jpeg
            && p.preview.is_none()
            && let Some(prepared) = &p.prepared
        {
            let step = prepared
                .width
                .div_ceil(300)
                .max(prepared.height.div_ceil(100))
                .max(1);
            let (w, h) = (
                prepared.width.div_ceil(step),
                prepared.height.div_ceil(step),
            );
            let mut bytes = vec![];
            for y in 0..h {
                for x in 0..w {
                    let n = (y * step * prepared.width + x * step) as usize;
                    if let Ok(sample) = prepared.sample(n, p.options.applied()) {
                        bytes.extend(sample.rgba8());
                    }
                }
            }
            if bytes.len() == w as usize * h as usize * 4 {
                p.preview = Some(ctx.load_texture(
                    "still-export-preview",
                    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &bytes),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        if browse
            && let Some(path) = rfd::FileDialog::new()
                .add_filter(p.options.format.label(), &[p.options.format.extension()])
                .set_file_name(format!("drawing.{}", p.options.format.extension()))
                .save_file()
        {
            p.path = path.display().to_string();
            p.replace = false;
        }
        if watermark {
            if let Err(e) = self.begin_output_batch() {
                p.message = e;
            } else {
                return;
            }
        }
        if export {
            match self.save_raster_export(&p) {
                Ok(()) => {
                    self.status = Some(format!("Exported {}", p.path));
                    return;
                }
                Err(e) => p.message = e,
            }
        }
        if open && !cancel && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.raster_export = p;
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    core::verify::verify(root)?;
    let dir = root.join("raster-export-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        let mut fixture = core::verify::fixture()?;
        fixture.layers.push(fixture.layers[0].clone());
        app.add_document(fixture);
        let source = app.documents[0].snapshot();
        app.begin_raster_export()?;
        app.raster_export.options.format = Format::Tiff;
        app.raster_export.options.depth = Depth::Sixteen;
        app.raster_export.options.alpha = true;
        app.raster_export.path = dir.join("out.tif").display().to_string();
        let mut t = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        for _ in 0..300 {
            frame(&mut app, &mut t, vec![]);
            if app.raster_export.prepared.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            let r = a
                .raster_export
                .controls
                .get(key)
                .copied()
                .ok_or("Missing raster export control")?;
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
            Ok(())
        };
        click(&mut app, &mut t, "confirm")?;
        if !app.raster_export.confirmed {
            return Err(format!(
                "Confirm click missed: {:?}; format={:?}, message={}",
                app.raster_export.controls, app.raster_export.options, app.raster_export.message
            ));
        }
        click(&mut app, &mut t, "export")?;
        if !dir.join("out.tif").is_file() || app.raster_export.open {
            return Err(format!(
                "Raster export pointer failed: {}",
                app.raster_export.message
            ));
        }
        if app.documents[0].snapshot() != source {
            return Err("Export UI changed the source".into());
        }
        app.begin_raster_export()?;
        click(&mut app, &mut t, "cancel")?;
        if app.raster_export.open {
            return Err("Cancel kept image export open".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}

impl Drawer {
    pub(super) fn quiesce_raster_export(&mut self, ctx: &egui::Context) -> bool {
        let open = self.raster_export.open;
        self.raster_export.open = false;
        self.raster_export_window(ctx);
        self.raster_export.open = open;
        self.raster_export.job.is_some()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_tiff_sixteen_bit_export_cancel_and_source_isolation() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}

#[path = "export_loss_verify.rs"]
mod loss_verify;
pub(super) fn loss_smoke(root: &std::path::Path) -> Result<(), String> {
    loss_verify::run(root)
}

#[path = "jpeg_output_verify.rs"]
mod jpeg_verify;
pub(super) fn jpeg_smoke(root: &std::path::Path) -> Result<(), String> {
    jpeg_verify::run(root)
}

#[path = "raster_channels_verify.rs"]
mod channels_verify;
pub(super) fn channels_smoke(root: &std::path::Path) -> Result<(), String> {
    channels_verify::run(root)
}
