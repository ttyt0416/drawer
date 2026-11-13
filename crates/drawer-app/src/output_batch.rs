use super::*;
use drawer_core::{
    color_precision::Depth,
    output_batch as core,
    raster_io::{Compression, Format},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    folder: String,
    settings: core::Settings,
    watermark_blocked: bool,
    watermark_replace: bool,
    all: bool,
    mode: drawer_core::layer_output::Mode,
    include_hidden: bool,
    reviewed: bool,
    plan: Option<Box<core::Plan>>,
    job: Option<Job>,
    report: Option<core::Report>,
    textures: Vec<egui::TextureHandle>,
    show_source: bool,
    texture_source: bool,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
enum Value {
    Plan(Box<core::Plan>),
    Report(core::Report),
}
struct Job {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Value, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn job(
    work: impl FnOnce(&AtomicBool) -> Result<Value, String> + Send + 'static,
) -> Result<Job, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("local-output-batch".into())
        .spawn(move || {
            let _ = tx.send(work(&flag));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { cancel, rx })
}
impl Drawer {
    pub(super) fn begin_layer_output(
        &mut self,
        mode: drawer_core::layer_output::Mode,
    ) -> Result<(), String> {
        self.begin_output_batch()?;
        self.output_batch.mode = mode;
        self.output_batch.settings.pattern = if mode == drawer_core::layer_output::Mode::Comps {
            "{title}-{index}-{comp}"
        } else {
            "{title}-{index}-{layer}"
        }
        .into();
        Ok(())
    }
    pub(super) fn begin_output_batch(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a source drawing")?];
        let saved = self
            .storage
            .as_ref()
            .map(drawer_core::output_watermark::Settings::load)
            .transpose();
        self.output_batch = Panel {
            open: true,
            settings: core::Settings {
                watermark: saved
                    .as_ref()
                    .ok()
                    .and_then(|v| v.clone())
                    .unwrap_or_default(),
                geometry: core::Geometry {
                    width: d.width,
                    height: d.height,
                    dpi: d.dpi,
                    ..Default::default()
                },
                ..Default::default()
            },
            watermark_blocked: saved.is_err(),
            message: saved.err().unwrap_or_default(),
            ..Default::default()
        };
        Ok(())
    }
    fn prepare_output_batch(&self, p: &mut Panel) -> Result<(), String> {
        if p.job.is_some() {
            return Err("An output job is running".into());
        }
        let indices = if p.all {
            (0..self.documents.len()).collect::<Vec<_>>()
        } else {
            vec![self.active.ok_or("Open a source drawing")?]
        };
        let held = indices
            .iter()
            .map(|n| self.documents[*n].memory_bytes())
            .sum::<usize>();
        let available = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        if indices.len() > 32 || held.saturating_mul(3) > available {
            return Err("Output source copies exceed workspace budget /32 documents".into());
        }
        let mut docs = Vec::new();
        for n in indices {
            let d = &self.documents[n];
            let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
            copy.id = d.id;
            copy.revision = d.revision;
            copy.title = d.title.clone();
            copy.path = d.path.clone();
            copy.source_path = d.source_path.clone();
            docs.push(copy);
        }
        let mode = p.mode;
        let hidden = p.include_hidden;
        let folder = PathBuf::from(&p.folder);
        let mut settings = p.settings.clone();
        p.plan = None;
        p.textures.clear();
        p.reviewed = false;
        p.report = None;
        p.job = Some(job(move |c| {
            settings.watermark = drawer_core::output_watermark::frozen(
                &settings.watermark,
                available.saturating_sub(held),
                c,
            )?;
            drawer_core::layer_output::prepare(
                &docs,
                &folder,
                &settings,
                mode,
                hidden,
                available.saturating_sub(held),
                c,
            )
            .map(|p| Value::Plan(Box::new(p)))
        })?);
        Ok(())
    }
    fn publish_output_batch(&self, p: &mut Panel) -> Result<(), String> {
        if p.job.is_some() || !p.reviewed {
            return Err("Review output names, sizes and notices first".into());
        }
        let plan = p.plan.as_ref().ok_or("Prepare output first")?;
        if !plan.matches(&self.documents) {
            return Err("Source document/frame changed or closed; prepare again".into());
        }
        plan.protect(&self.protected_sources)?;
        let plan = p.plan.take().unwrap();
        p.job = Some(job(move |c| Ok(Value::Report(plan.publish(c))))?);
        p.reviewed = false;
        Ok(())
    }
    pub(super) fn output_batch_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.output_batch);

        if let Some(j) = &p.job {
            match j.rx.try_recv() {
                Ok(r) => {
                    p.job = None;
                    match r {
                        Ok(Value::Plan(v)) => {
                            p.message="Prepared names and actual encoded byte counts; review before writing".into();
                            p.plan = Some(v);
                        }
                        Ok(Value::Report(r)) => {
                            let good = r.files.iter().filter(|f| f.result.is_ok()).count();
                            p.message = format!(
                                "{good}/{} new files written. Existing files and source documents unchanged.",
                                r.files.len()
                            );
                            p.report = Some(r);
                        }
                        Err(e) => p.message = e,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    p.job = None;
                    p.message = "Output worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(25))
                }
            }
        }
        if !p.open {
            self.output_batch = p;
            return;
        }
        if (p.textures.is_empty() || p.texture_source != p.show_source)
            && let Some(plan) = &p.plan
        {
            p.textures.clear();
            p.texture_source = p.show_source;
            for f in &plan.files {
                let pixels = if p.show_source {
                    &f.source_preview
                } else {
                    &f.preview
                };
                let bytes = pixels.iter().flatten().copied().collect::<Vec<_>>();
                p.textures.push(ctx.load_texture(
                    f.path.display().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(f.preview_size, &bytes),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        let before = (
            p.settings.clone(),
            p.folder.clone(),
            p.all,
            p.mode,
            p.include_hidden,
        );
        let mut open = true;
        let mut action = "";
        let mut close = false;
        p.controls.clear();
        egui::Window::new("Output size, resolution and filename plan").default_pos(egui::pos2(25.,25.)).default_width(640.).max_height(680.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.horizontal(|ui|{for(key,label,enabled)in[("prepare","Prepare preview",p.job.is_none()),("write","Write reviewed new files",p.job.is_none()&&p.plan.is_some()&&p.reviewed)]{let b=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key,b.rect);if b.clicked(){action=key;}}let b=ui.button(crate::i18n::text("Close"));p.controls.insert("close",b.rect);close=b.clicked();});
            let b=ui.checkbox(&mut p.show_source,"Show source sRGB instead of output soft proof (view only)");p.controls.insert("source-preview",b.rect);
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                ui.horizontal(|ui|{ui.label("Local output folder");ui.text_edit_singleline(&mut p.folder);if ui.button(crate::i18n::text("Browse…")).clicked()&&let Some(f)=rfd::FileDialog::new().pick_folder(){p.folder=f.display().to_string();}});
                ui.checkbox(&mut p.all,"All open documents (max32); otherwise active document");
                ui.horizontal(|ui|{for(key,label,mode)in[("composite","Full composite",drawer_core::layer_output::Mode::Composite),("layers","Each layer",drawer_core::layer_output::Mode::Layers),("comps","Each saved comp",drawer_core::layer_output::Mode::Comps)]{let r=ui.selectable_value(&mut p.mode,mode,label);p.controls.insert(key,r.rect);}});
                if p.mode==drawer_core::layer_output::Mode::Layers{ui.checkbox(&mut p.include_hidden,"Include hidden layers");}
                if p.mode!=drawer_core::layer_output::Mode::Composite{ui.small("Up to128 outputs. Add {layer}/{comp} and {index} to the filename; forbidden names/collisions are rejected. Independent layer output has no lower backdrop.");}
                ui.horizontal(|ui|{ui.label("Filename stem");ui.text_edit_singleline(&mut p.settings.pattern);});ui.small("{title} = source title without extension; {index} =0001…; chosen format supplies the extension. Existing files are reported as failures and preserved.");
                ui.horizontal_wrapped(|ui|{for f in Format::ALL{ui.selectable_value(&mut p.settings.encoding.format,f,f.extension());}});
                let g=&mut p.settings.geometry;ui.checkbox(&mut g.resize,"Custom output pixels (source size otherwise)");ui.add_enabled_ui(g.resize,|ui|{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut g.width).range(0..=32768).prefix("Width "));ui.add(egui::DragValue::new(&mut g.height).range(0..=32768).prefix("Height "));});});
                ui.checkbox(&mut g.resolution,"Override output DPI (source DPI otherwise)");ui.add_enabled(g.resolution,egui::DragValue::new(&mut g.dpi).range(0.0..=4800.0).prefix("DPI "));
                let o=&mut p.settings.encoding;
                ui.checkbox(&mut o.grayscale,"Grayscale output (PNG Gray/GrayAlpha; others equal RGB)");
                if o.format==Format::Jpeg{ui.add(egui::Slider::new(&mut o.quality,1..=100).text("JPEG luma quality"));ui.add(egui::Slider::new(&mut o.chroma_quality,1..=100).text("JPEG chroma quality"));ui.horizontal(|ui|{for v in[drawer_core::jpeg_output::Sampling::S444,drawer_core::jpeg_output::Sampling::S422,drawer_core::jpeg_output::Sampling::S420]{ui.selectable_value(&mut o.sampling,v,v.label());}});ui.small("Baseline sequential RGB8; progressive/arithmetic/custom tables are unavailable in this output adapter.");}else{ui.checkbox(&mut o.alpha,if o.format==Format::Bmp{"BMP32 alpha (V5); off = BMP24 opaque (INFO)"}else{"Transparent output"});}
                ui.add_enabled_ui(!o.applied().alpha,|ui|{ui.horizontal(|ui|{ui.label("Opaque background");ui.color_edit_button_srgb(&mut o.background);});});
                if matches!(o.format,Format::Png|Format::Tiff){ui.horizontal(|ui|{ui.selectable_value(&mut o.depth,Depth::Eight,"8bit");ui.selectable_value(&mut o.depth,Depth::Sixteen,"16bit");});

                    ui.horizontal(|ui|{for c in [Compression::None,Compression::Deflate,Compression::Lzw,Compression::Packbits]{if o.format==Format::Png&&!matches!(c,Compression::None|Compression::Deflate){continue;}ui.selectable_value(&mut o.compression,c,format!("{c:?}"));}});
                }
                if o.format==Format::Tga{ui.checkbox(&mut o.rle,"TGA RLE");}
                ui.separator();let color=&mut p.settings.color;
                ui.add_enabled(!o.grayscale&&o.format!=Format::Bmp,egui::Checkbox::new(&mut color.convert,"Convert pixel RGB to the output profile"));
                ui.add_enabled(drawer_core::export_color::supports_profile(o.format),egui::Checkbox::new(&mut color.embed,"Include ICC profile (separate from pixel conversion)"));
                if !drawer_core::export_color::supports_profile(o.format){ui.small("This format adapter cannot embed ICC. The stored include choice is preserved but inactive.");}
                ui.add_enabled_ui(!o.grayscale&&o.format!=Format::Bmp&&(color.convert||color.embed&&drawer_core::export_color::supports_profile(o.format)),|ui|{
                    let mut value=color.profile.as_ref().map_or(String::new(),|v|v.display().to_string());
                    ui.horizontal(|ui|{ui.label("Output RGB ICC");if ui.text_edit_singleline(&mut value).changed(){color.profile=(!value.trim().is_empty()).then(||PathBuf::from(value.trim()));}
                        if ui.button("ICC…").clicked()&&let Some(f)=rfd::FileDialog::new().add_filter("RGB ICC",&["icc","icm"]).pick_file(){color.profile=Some(f);}
                        if ui.button("Default profile").clicked(){color.profile=None;}
                    });
                });
                if o.grayscale||o.format==Format::Bmp{ui.small("Grayscale/BMP uses standard sRGB colorimetry; RGB target/conversion/intent choices are preserved but inactive. Gray PNG embeds a matching Gray ICC when selected.");}
                ui.small("Default: sRGB for conversion; document working RGB for embed-only. Embed-only leaves numeric RGB unchanged and may change interpretation. CMYK output is unsupported; CMYK canvas soft proof remains separate.");
                ui.add_enabled_ui(!o.grayscale&&o.format!=Format::Bmp&&color.convert,|ui|{ui.horizontal_wrapped(|ui|{for(intent,label)in[(drawer_core::soft_proof::Intent::Perceptual,"Perceptual"),(drawer_core::soft_proof::Intent::Relative,"Relative Colorimetric"),(drawer_core::soft_proof::Intent::Saturation,"Saturation"),(drawer_core::soft_proof::Intent::Absolute,"Absolute Colorimetric")]{ui.selectable_value(&mut color.intent,intent,label);}});});

                ui.collapsing("Output watermark / independent noise",|ui|{crate::output_watermark::editor(ui,&mut p.settings.watermark,&mut p.message);if p.watermark_blocked{ui.checkbox(&mut p.watermark_replace,"Replace unreadable saved watermark preferences");}
                if ui.add_enabled(!p.watermark_blocked||p.watermark_replace,egui::Button::new("Save local watermark preset")).clicked(){p.message=match self.storage.as_ref().ok_or_else(||"Local storage unavailable".to_string()).and_then(|v|p.settings.watermark.save(v)){Ok(())=>{p.watermark_blocked=false;"Local preset saved; closing without Save preserves the previous preset".into()},Err(e)=>e};}});
                ui.checkbox(&mut p.reviewed,"Reviewed output names, pixel/DPI settings, losses and output details");
            });
            if p.job.is_some(){ui.spinner();if ui.button(crate::i18n::text("Cancel job")).clicked(){if let Some(j)=&p.job{j.cancel.store(true,Ordering::Relaxed);}p.message="Cancel requested. Completed new files remain; pending files are reported canceled.".into();}}
            ui.label(&p.message);
            if let Some(plan)=&p.plan{for(f,t)in plan.files.iter().zip(&p.textures){ui.separator();ui.label(format!("{} · {}×{} · {:.5}DPI · {} bytes",f.path.display(),f.width,f.height,f.dpi,f.bytes.len()));ui.image((t.id(),t.size_vec2()));if !f.losses.is_empty(){ui.colored_label(egui::Color32::YELLOW,"Losses in this output; keep the editable source");for loss in &f.losses{ui.label(loss);}}for note in &f.notices{ui.small(note);}}}
            if let Some(r)=&p.report{for f in &r.files{ui.label(format!("{}: {}",f.path.display(),match &f.result{Ok(())=>"Written".into(),Err(e)=>format!("Failed / skipped: {e}")}));}}
        });
        if before
            != (
                p.settings.clone(),
                p.folder.clone(),
                p.all,
                p.mode,
                p.include_hidden,
            )
        {
            p.plan = None;
            p.textures.clear();
            p.reviewed = false;
        }
        let result = match action {
            "prepare" => self.prepare_output_batch(&mut p),
            "write" => self.publish_output_batch(&mut p),
            _ => Ok(()),
        };
        if let Err(e) = result {
            p.message = e;
        }
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.job = None;
            p.plan = None;
            p.textures.clear();
            p.reviewed = false;
        }
        self.output_batch = p;
    }
}
#[path = "output_batch_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}

impl Drawer {
    pub(super) fn quiesce_output_batch(&mut self, ctx: &egui::Context) -> bool {
        let open = self.output_batch.open;
        self.output_batch.open = false;
        self.output_batch_window(ctx);
        self.output_batch.open = open;
        self.output_batch.job.is_some()
    }
}

impl Drawer {
    pub(super) fn rebase_output_batch(&mut self, old: &Path, new: &Path) -> Result<(), String> {
        self.output_batch.settings.watermark = drawer_core::storage_migration::rebase_settings(
            &self.output_batch.settings.watermark,
            old,
            new,
        )?;
        if let Some(path) = &self.output_batch.settings.color.profile {
            let value = drawer_core::storage_migration::rebase_settings(path, old, new)?;
            self.output_batch.settings.color.profile = Some(value);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn output_pointer_prepare_cancel_publish_and_source_guard() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}

#[path = "output_watermark_verify.rs"]
mod watermark_verify;
pub(super) fn watermark_smoke(root: &Path) -> Result<(), String> {
    watermark_verify::run(root)
}

#[path = "layer_output_verify.rs"]
mod layer_verify;
pub(super) fn layer_smoke(root: &Path) -> Result<(), String> {
    layer_verify::run(root)
}
