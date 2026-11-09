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
    all: bool,
    reviewed: bool,
    plan: Option<Box<core::Plan>>,
    job: Option<Job>,
    report: Option<core::Report>,
    textures: Vec<egui::TextureHandle>,
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
    pub(super) fn begin_output_batch(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a source drawing")?];
        self.output_batch = Panel {
            open: true,
            settings: core::Settings {
                geometry: core::Geometry {
                    width: d.width,
                    height: d.height,
                    dpi: d.dpi,
                    ..Default::default()
                },
                ..Default::default()
            },
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
        let folder = PathBuf::from(&p.folder);
        let settings = p.settings.clone();
        p.plan = None;
        p.textures.clear();
        p.reviewed = false;
        p.report = None;
        p.job = Some(job(move |c| {
            core::prepare(&docs, &folder, &settings, available.saturating_sub(held), c)
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
        if !p.open {
            self.output_batch = p;
            return;
        }
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
        if p.textures.is_empty()
            && let Some(plan) = &p.plan
        {
            for f in &plan.files {
                let bytes = f.preview.iter().flatten().copied().collect::<Vec<_>>();
                p.textures.push(ctx.load_texture(
                    f.path.display().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(f.preview_size, &bytes),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        let before = (p.settings.clone(), p.folder.clone(), p.all);
        let mut open = true;
        let mut action = "";
        let mut close = false;
        p.controls.clear();
        egui::Window::new("Output size, resolution and filename plan").default_pos(egui::pos2(25.,25.)).default_width(640.).max_height(680.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.horizontal(|ui|{for(key,label,enabled)in[("prepare","Prepare preview",p.job.is_none()),("write","Write reviewed new files",p.job.is_none()&&p.plan.is_some()&&p.reviewed)]{let b=ui.add_enabled(enabled,egui::Button::new(label));p.controls.insert(key,b.rect);if b.clicked(){action=key;}}let b=ui.button("Close");p.controls.insert("close",b.rect);close=b.clicked();});
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                ui.horizontal(|ui|{ui.label("Local output folder");ui.text_edit_singleline(&mut p.folder);if ui.button("Browse…").clicked()&&let Some(f)=rfd::FileDialog::new().pick_folder(){p.folder=f.display().to_string();}});
                ui.checkbox(&mut p.all,"All open documents (max32); otherwise active document");
                ui.horizontal(|ui|{ui.label("Filename stem");ui.text_edit_singleline(&mut p.settings.pattern);});ui.small("{title} = source title without extension; {index} =0001…; chosen format supplies the extension. Existing files are reported as failures and preserved.");
                ui.horizontal_wrapped(|ui|{for f in Format::ALL{ui.selectable_value(&mut p.settings.encoding.format,f,f.extension());}});
                let g=&mut p.settings.geometry;ui.checkbox(&mut g.resize,"Custom output pixels (source size otherwise)");ui.add_enabled_ui(g.resize,|ui|{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut g.width).range(0..=32768).prefix("Width "));ui.add(egui::DragValue::new(&mut g.height).range(0..=32768).prefix("Height "));});});
                ui.checkbox(&mut g.resolution,"Override output DPI (source DPI otherwise)");ui.add_enabled(g.resolution,egui::DragValue::new(&mut g.dpi).range(0.0..=4800.0).prefix("DPI "));
                let o=&mut p.settings.encoding;
                if o.format==Format::Jpeg{o.alpha=false;o.depth=Depth::Eight;ui.add(egui::Slider::new(&mut o.quality,1..=100).text("JPEG quality"));}else{ui.checkbox(&mut o.alpha,"Transparent output");}
                ui.add_enabled_ui(!o.alpha,|ui|{ui.horizontal(|ui|{ui.label("Opaque background");ui.color_edit_button_srgb(&mut o.background);});});
                if matches!(o.format,Format::Png|Format::Tiff){ui.horizontal(|ui|{ui.selectable_value(&mut o.depth,Depth::Eight,"8bit");ui.selectable_value(&mut o.depth,Depth::Sixteen,"16bit");});
                    if o.format==Format::Png&&!matches!(o.compression,Compression::None|Compression::Deflate){o.compression=Compression::Deflate;}
                    ui.horizontal(|ui|{for c in [Compression::None,Compression::Deflate,Compression::Lzw,Compression::Packbits]{if o.format==Format::Png&&!matches!(c,Compression::None|Compression::Deflate){continue;}ui.selectable_value(&mut o.compression,c,format!("{c:?}"));}});
                }else{o.depth=Depth::Eight;}
                if o.format==Format::Tga{ui.checkbox(&mut o.rle,"TGA RLE");}
                ui.checkbox(&mut p.reviewed,"Reviewed output names, pixel/DPI settings and conversion notices");
            });
            if p.job.is_some(){ui.spinner();if ui.button("Cancel job").clicked(){if let Some(j)=&p.job{j.cancel.store(true,Ordering::Relaxed);}p.message="Cancel requested. Completed new files remain; pending files are reported canceled.".into();}}
            ui.label(&p.message);
            if let Some(plan)=&p.plan{for(f,t)in plan.files.iter().zip(&p.textures){ui.separator();ui.label(format!("{} · {}×{} · {:.5}DPI · {} bytes",f.path.display(),f.width,f.height,f.dpi,f.bytes.len()));ui.image((t.id(),t.size_vec2()));for note in &f.notices{ui.small(note);}}}
            if let Some(r)=&p.report{for f in &r.files{ui.label(format!("{}: {}",f.path.display(),match &f.result{Ok(())=>"Written".into(),Err(e)=>format!("Failed / skipped: {e}")}));}}
        });
        if before != (p.settings.clone(), p.folder.clone(), p.all) {
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
#[cfg(test)]
mod tests {
    #[test]
    fn output_pointer_prepare_cancel_publish_and_source_guard() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
