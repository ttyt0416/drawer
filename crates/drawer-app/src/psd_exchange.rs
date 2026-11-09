use super::*;
use drawer_core::psd_exchange::{self as core, Format, Plan};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
pub(super) struct Export {
    id: u64,
    revision: u64,
    frame: u32,
    path: PathBuf,
    format: Format,
    replace: bool,
    confirmed: bool,
    flatten: bool,
    open: bool,
    plan: Option<Plan>,
    preview: Option<egui::TextureHandle>,
    cancel: Arc<AtomicBool>,
    preparing: Option<mpsc::Receiver<Result<Plan, String>>>,
    writing: Option<mpsc::Receiver<(Plan, Result<(), String>)>>,
    message: String,
    pub(super) controls: BTreeMap<&'static str, egui::Rect>,
}
impl Drawer {
    pub(super) fn begin_psd_exchange(&mut self, path: PathBuf) -> Result<(), String> {
        if self.textures.export.is_some() {
            return Err("Finish or cancel the current PSD export first".into());
        }
        if self.drawing.has_path_draft() {
            return Err("Commit or cancel the path draft before exporting".into());
        }
        self.finish_stroke();
        let n = self.active.ok_or("Open a document first")?;
        let d = &self.documents[n];
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let available =
            DEFAULT_DOCUMENT_BUDGET.saturating_sub(used.saturating_sub(d.resident_bytes()));
        if d.memory_bytes()
            .saturating_mul(8)
            .saturating_add(d.resident_bytes())
            > available
        {
            return Err("PSD preparation exceeds workspace memory".into());
        }
        let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        copy.id = d.id;
        copy.revision = d.revision;
        copy.path = d.path.clone();
        copy.source_path = d.source_path.clone();
        let format = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("psb"))
        {
            Format::Psb
        } else {
            Format::Psd
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("prepare-psd-exchange".into())
            .spawn(move || {
                let _ = tx.send(core::prepare(&copy, available, &worker));
            })
            .map_err(|e| e.to_string())?;
        self.textures.export = Some(Export {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            path,
            format,
            replace: false,
            confirmed: false,
            flatten: false,
            open: true,
            plan: None,
            preview: None,
            cancel,
            preparing: Some(rx),
            writing: None,
            message: "Preparing layer conversion and composite preview…".into(),
            controls: BTreeMap::new(),
        });
        Ok(())
    }
    fn write_psd_exchange(&mut self, e: &mut Export) -> Result<(), String> {
        if e.writing.is_some() || e.preparing.is_some() {
            return Err("An export job is already running".into());
        }
        let n = self.active.ok_or("Open the source document")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Source document/frame changed; prepare the export again".into());
        }
        if !e.confirmed {
            return Err("Review and confirm the listed conversion losses".into());
        }
        let target = self.export_target(n, &e.path)?;
        let available = DEFAULT_DOCUMENT_BUDGET.saturating_sub(
            self.documents
                .iter()
                .map(Document::resident_bytes)
                .sum::<usize>(),
        );
        let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        copy.id = d.id;
        copy.revision = d.revision;
        copy.path = d.path.clone();
        copy.source_path = d.source_path.clone();
        e.plan.as_ref().ok_or("Wait for the preview")?.check(d)?;
        let (format, flatten, confirmed, replace) = (e.format, e.flatten, e.confirmed, e.replace);
        let cancel = e.cancel.clone();
        let (tx, rx) = mpsc::channel();
        // The named thread is started before handing off the plan so spawn failure cannot consume the preview.
        let (job_tx, job_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("write-psd-exchange".into())
            .spawn(move || {
                if let Ok(plan) = job_rx.recv() {
                    let plan: Plan = plan;
                    let result = plan.write(
                        &copy, &target, format, flatten, confirmed, replace, available, &cancel,
                    );
                    let _ = tx.send((plan, result));
                }
            })
            .map_err(|err| err.to_string())?;
        let plan = e
            .plan
            .take()
            .ok_or("PSD preview ownership transfer failed")?;
        if let Err(err) = job_tx.send(plan) {
            e.plan = Some(err.0);
            return Err("PSD write worker stopped before receiving the plan".into());
        }
        e.writing = Some(rx);
        e.message = "Writing local exchange file…".into();
        Ok(())
    }
    fn poll_psd_exchange(&mut self, e: &mut Export, ctx: &egui::Context) -> bool {
        if let Some(rx) = &e.preparing {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("PSD preparation worker stopped".into())),
            };
            if let Some(result) = result {
                e.preparing = None;
                if e.cancel.load(Ordering::Relaxed) {
                    return true;
                }
                match result {
                    Err(err) => e.message = err,
                    Ok(plan) => {
                        if self
                            .active
                            .is_none_or(|n| plan.check(&self.documents[n]).is_err())
                        {
                            e.message =
                                "Source changed during preparation; cancel and prepare again"
                                    .into();
                        } else {
                            let image = &plan.flattened;
                            let bytes = image.merged.iter().flat_map(|p| *p).collect::<Vec<_>>();
                            e.preview = Some(ctx.load_texture(
                                "psd-exchange-preview",
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [image.width as usize, image.height as usize],
                                    &bytes,
                                ),
                                egui::TextureOptions::LINEAR,
                            ));
                            e.flatten = plan.partial.is_none();
                            e.message = format!(
                                "Preview validated; partial composite difference {} RGBA8 level(s)",
                                plan.delta
                            );
                            e.plan = Some(plan);
                        }
                    }
                }
            }
        }
        if let Some(rx) = &e.writing {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => {
                    e.message =
                        "PSD write worker stopped; inspect the destination before retrying".into();
                    e.writing = None;
                    None
                }
            };
            if let Some((plan, result)) = result {
                e.writing = None;
                match result {
                    Ok(()) => {
                        self.status = Some(format!("PSD/PSB exported: {}", e.path.display()));
                        return true;
                    }
                    Err(err) => {
                        e.message = err;
                        e.plan = Some(plan);
                        if e.cancel.load(Ordering::Relaxed) {
                            return true;
                        }
                    }
                }
            }
        }
        !e.open && e.preparing.is_none() && e.writing.is_none()
    }
    pub(super) fn psd_import_report_window(&mut self, ctx: &egui::Context) {
        if let Some(lines) = &self.textures.report {
            let mut open = true;
            egui::Window::new("PSD / PSB import results")
                .default_width(650.)
                .max_height(580.)
                .vscroll(true)
                .open(&mut open)
                .show(ctx, |ui| {
                    for line in lines {
                        ui.label(line);
                    }
                });
            if !open {
                self.textures.report = None;
            }
        }
    }
    pub(super) fn psd_exchange_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.textures.export.take() else {
            return;
        };
        if self.poll_psd_exchange(&mut e, ctx) {
            return;
        }
        let mut open = e.open;
        let mut write = false;
        let mut cancel = false;
        e.controls.clear();
        if open {
            egui::Window::new("PSD / PSB exchange preview").default_pos(egui::pos2(25.,25.)).default_width(650.).max_height(640.).vscroll(true).open(&mut open).show(ctx,|ui|{
            ui.label(&e.message);ui.horizontal(|ui|{let busy=e.preparing.is_some()||e.writing.is_some();let r=ui.add_enabled(!busy&&e.plan.is_some()&&e.confirmed,egui::Button::new("Export confirmed result"));e.controls.insert("export",r.rect);write=r.clicked();let r=ui.button("Cancel");e.controls.insert("cancel",r.rect);cancel=r.clicked();});
            ui.add_enabled_ui(e.writing.is_none(),|ui|{let mut path=e.path.display().to_string();if ui.text_edit_singleline(&mut path).changed(){e.path=PathBuf::from(path);e.replace=false;e.confirmed=false;}
                ui.horizontal(|ui|{for(f,name)in[(Format::Psd,"PSD"),(Format::Psb,"PSB")]{let r=ui.selectable_value(&mut e.format,f,name);e.controls.insert(name,r.rect);if r.changed(){e.path.set_extension(if f==Format::Psd{"psd"}else{"psb"});e.replace=false;e.confirmed=false;}}});
                if let Some(p)=&e.plan{let partial=ui.add_enabled(p.partial.is_some(),egui::RadioButton::new(!e.flatten,"Preserve verified layers / minimal dependency ranges"));e.controls.insert("partial",partial.rect);if partial.clicked(){e.flatten=false;e.confirmed=false;}let r=ui.radio(e.flatten,"Whole composite — all separate layer/group/mask/object editing lost");e.controls.insert("whole",r.rect);if r.clicked(){e.flatten=true;e.confirmed=false;}
                    let r=ui.checkbox(&mut e.confirmed,"I reviewed the conversion ranges and editing losses below");e.controls.insert("confirm",r.rect);
                    ui.checkbox(&mut e.replace,"Replace this existing export file (original source is always protected)");
                    egui::ScrollArea::vertical().id_salt("psd-losses").max_height(145.).show(ui,|ui|{if let Ok(report)=p.report(e.format,e.flatten){for line in report.lines(){ui.label(line);}}
if e.flatten{ui.colored_label(egui::Color32::YELLOW,format!("Whole composite range: {}",p.whole_names.join(" → ")));}else{for range in &p.ranges{ui.colored_label(egui::Color32::YELLOW,format!("Rasterize range: {}",range.join(" → ")));}}
if let Some(reason)=&p.reason{ui.colored_label(egui::Color32::YELLOW,reason);}});
                }
            });
            if let Some(preview)=&e.preview{let size=preview.size_vec2();let scale=(350./size.x).min(180./size.y).min(1.);ui.image((preview.id(),size*scale));}
            ui.small("RGB8 sRGB; raw PSD/PSB output. Import supports raw / PackBits RLE RGB8. Text/smart-object/effect caches are validated against the merged preview; unsupported color/compression variants are refused. The .drawer source and its Undo history stay unchanged.");
        });
        }
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            e.cancel.store(true, Ordering::Relaxed);
            e.open = false;
            if e.preparing.is_none() && e.writing.is_none() {
                return;
            }
        }
        if write && let Err(err) = self.write_psd_exchange(&mut e) {
            e.message = err;
        }
        self.textures.export = Some(e);
    }
}
#[path = "psd_exchange_verify.rs"]
mod verify;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn pointer_confirm_export_cancel_stale_and_source_unchanged() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
