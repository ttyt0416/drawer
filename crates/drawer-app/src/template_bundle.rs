use super::*;
use drawer_core::template_bundle::{self as core, Kind};
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
    path: String,
    name: String,
    kind: Kind,
    copy_masks: bool,
    reviewed: bool,
    prepared: Option<Box<core::Prepared>>,
    job: Option<Job>,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
enum Value {
    Exported,
    Imported(Box<core::Prepared>),
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
impl Drawer {
    pub(super) fn begin_template_bundle(&mut self) {
        self.finish_stroke();
        self.template_bundle.open = true;
    }
    fn template_job(&mut self, p: &mut Panel, export: bool) -> Result<(), String> {
        if p.job.is_some() {
            return Err("A template job is running".into());
        }
        let path = PathBuf::from(&p.path);
        drawer_core::file_objects::local_path(&path)?;
        let budget = DEFAULT_DOCUMENT_BUDGET
            .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum());
        let copy = if export {
            let d = &self.documents[self.active.ok_or("Open a source page first")?];
            if d.memory_bytes().saturating_mul(3) > budget {
                return Err("Template snapshot exceeds workspace memory".into());
            }
            let mut copy = drawer_core::layer_merge::preview(&d.snapshot(), d.current_frame)?;
            copy.light_table = d.light_table.clone();
            Some(copy)
        } else {
            None
        };
        let masks = self.materials.library.clone();
        let name = p.name.clone();
        let kind = p.kind;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("local-template-bundle".into())
            .spawn(move || {
                let result = if let Some(d) = copy {
                    core::export(&d, &path, &name, kind, &masks, &flag).map(|()| Value::Exported)
                } else {
                    core::import(&path, budget, &flag).map(|p| Value::Imported(Box::new(p)))
                };
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        p.prepared = None;
        p.reviewed = false;
        p.job = Some(Job { cancel, rx });
        p.message = "Validating local template resources…".into();
        Ok(())
    }
    fn accept_template(&mut self, p: &mut Panel) -> Result<(), String> {
        if p.job.is_some() || !p.reviewed {
            return Err("Review the complete template before applying".into());
        }
        let prepared = p.prepared.as_ref().ok_or("Prepare a template first")?;
        if self.tabs.len() >= 256
            || prepared.document.resident_bytes()
                > DEFAULT_DOCUMENT_BUDGET
                    .saturating_sub(self.documents.iter().map(Document::resident_bytes).sum())
        {
            return Err("Template no longer fits workspace memory/view limits".into());
        }
        self.documents.try_reserve(1).map_err(|e| e.to_string())?;
        self.tabs.try_reserve(1).map_err(|e| e.to_string())?;
        if p.copy_masks {
            let next = prepared.copied_masks(&self.materials.library)?;
            self.publish_materials(next)?;
        }
        let mut prepared = *p.prepared.take().unwrap();
        prepared.document.view = self.default_view;
        self.protected_sources
            .insert(prepared.source.join("template.drawer"));
        self.add_document(prepared.document);
        p.message="Opened an independent unsaved template document; source and previous drawings unchanged".into();
        p.reviewed = false;
        Ok(())
    }
    pub(super) fn template_bundle_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.template_bundle);
        if !p.open {
            self.template_bundle = p;
            return;
        }
        if let Some(job) = &p.job {
            match job.rx.try_recv() {
                Ok(r) => {
                    p.job = None;
                    match r {
                        Ok(Value::Exported) => {
                            p.message =
                                "New local template folder published; source unchanged".into()
                        }
                        Ok(Value::Imported(value)) => {
                            p.message =
                                "All resources validated. Review before creating a new document."
                                    .into();
                            p.prepared = Some(value);
                        }
                        Err(e) => p.message = e,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    p.job = None;
                    p.message = "Template worker stopped; existing document unchanged".into();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(25))
                }
            }
        }
        let mut open = true;
        let mut close = false;
        let mut action = "";
        p.controls.clear();
        egui::Window::new("Local templates and resource bundles").open(&mut open).default_width(570.).show(ctx,|ui|{
            ui.label("Editable full-page snapshots preserve layer stacks, comic cuts and timelines together.");
            ui.label("Native image/cel/model/audio/font caches are included. Linked sources refresh only on explicit request.");
            ui.add_enabled_ui(p.job.is_none(),|ui|{
                ui.horizontal(|ui|{ui.label("Folder");if ui.text_edit_singleline(&mut p.path).changed(){p.prepared=None;p.reviewed=false;}
                    if ui.button("Browse existing…").clicked() && let Some(f)=rfd::FileDialog::new().pick_folder(){p.path=f.display().to_string();p.prepared=None;p.reviewed=false;}});
                ui.horizontal(|ui|{ui.label("Template name");ui.text_edit_singleline(&mut p.name);});
                ui.horizontal(|ui|{for kind in Kind::ALL{ui.selectable_value(&mut p.kind,kind,kind.label());}});
                ui.label("Export: enter a NEW folder path. Import: choose a previously exported folder.");
                ui.horizontal(|ui|{for(key,label)in[("export","Export new bundle"),("prepare","Validate import")]{let b=ui.button(label);p.controls.insert(key,b.rect);if b.clicked(){action=key;}}});
                if let Some(v)=&p.prepared{
                    ui.separator();ui.label(format!("{} · {} · {}×{} · {:.2}DPI · {} raster layers · {} frames",v.name,v.kind.label(),v.document.width,v.document.height,v.document.dpi,v.document.layers.len(),v.document.animation.as_ref().map_or(1,|a|a.frames)));
                    ui.label(format!("{} archived mask definitions. Existing frozen stroke IDs/settings stay unchanged.",v.masks.assets().len()));
                    ui.checkbox(&mut p.copy_masks,"Also copy archived masks into my local library with new IDs");
                    ui.checkbox(&mut p.reviewed,"Reviewed: open a new unsaved document, preserving existing documents");
                    let b=ui.add_enabled(p.reviewed,egui::Button::new("Create from template"));p.controls.insert("apply",b.rect);if b.clicked(){action="apply";}
                }
            });
            if p.job.is_some(){ui.spinner();let b=ui.button("Cancel job");p.controls.insert("cancel-job",b.rect);if b.clicked(){if let Some(j)=&p.job{j.cancel.store(true,Ordering::Relaxed);}p.message="Cancellation requested; waiting for owned temporary cleanup".into();}}
            ui.label(&p.message);let b=ui.button("Close / discard preview");p.controls.insert("close",b.rect);if b.clicked(){close=true;}
        });
        let result = match action {
            "export" => self.template_job(&mut p, true),
            "prepare" => self.template_job(&mut p, false),
            "apply" => self.accept_template(&mut p),
            _ => Ok(()),
        };
        if let Err(e) = result {
            p.message = e;
        }
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.job = None;
            p.prepared = None;
            p.reviewed = false;
        }
        self.template_bundle = p;
    }
}
#[path = "template_bundle_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn template_pointer_review_cancel_and_new_document() {
        let t = crate::tests::TestDirectory::new();
        super::verify::run(&t.0).unwrap();
    }
}
