use super::*;
use drawer_core::storage_migration as core;
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
};
enum Value {
    Moved(core::Outcome),
    Restored,
    Recycled(PathBuf),
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    rx: mpsc::Receiver<Result<Value, String>>,
    handle: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub(super) open: bool,
    destination: String,
    pending: bool,
    reset_index: bool,
    acknowledged: bool,
    job: Option<Job>,
    message: String,
    commands: VecDeque<String>,
    drops: Vec<PathBuf>,
    backup: Option<PathBuf>,
    cleanup: Option<PathBuf>,
    confirm_cleanup: bool,
    controls: BTreeMap<&'static str, egui::Rect>,
}
impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.pending || self.job.is_some()
    }
    pub(super) fn defer(&mut self, command: &str) -> bool {
        if !self.busy() {
            return false;
        }
        if self.commands.len() < 128 {
            self.commands.push_back(command.into());
        } else {
            self.message = "Deferred command limit128; wait for the data operation".into();
        }
        true
    }
}
fn worker(
    work: impl FnOnce(Arc<AtomicBool>, Arc<AtomicUsize>) -> Result<Value, String> + Send + 'static,
) -> Result<Job, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AtomicUsize::new(0));
    let (c, p) = (cancel.clone(), progress.clone());
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("local-data-maintenance".into())
        .spawn(move || {
            let _ = tx.send(work(c, p));
        })
        .map_err(|e| e.to_string())?;
    Ok(Job {
        cancel,
        progress,
        rx,
        handle: Some(handle),
    })
}
impl Drawer {
    fn quiesce_data_io(&mut self, ctx: &egui::Context) -> bool {
        let mut busy = false;
        busy |= self.quiesce_asset_catalog(ctx);
        busy |= self.quiesce_raster_export(ctx);
        busy |= self.quiesce_output_batch(ctx);
        busy |= self.quiesce_book_io(ctx);
        busy |= self.quiesce_printing(ctx);
        busy |= self.quiesce_animation_export(ctx);
        busy |= self.quiesce_action_runner(ctx);
        busy |= self.quiesce_action_repeat(ctx);
        busy |= self.quiesce_auto_actions(ctx);
        busy |= self.quiesce_work_library(ctx);
        busy |= self.quiesce_file_browser(ctx);
        busy |= self.quiesce_timelapse(ctx);
        busy
    }
    fn moved_data_root(&mut self, out: core::Outcome) -> Result<String, String> {
        let new = out.storage.root().to_owned();
        let old = out.old.clone();
        self.storage = Some(out.storage);
        let mut notices = vec![];
        for source in core::asset_roots(&old) {
            let target = drawer_core::file_browser::remap(&source, &old, &new)
                .ok_or("Managed asset root")?;
            for d in &mut self.documents {
                d.remap_managed_files(&source, &target);
            }
            if let Err(e) =
                self.references
                    .session
                    .remap_sources(&source, &target, self.storage.as_ref())
            {
                notices.push(e.to_string());
            }
        }
        for result in [
            self.rebase_auto_actions(&old, &new),
            self.rebase_output_batch(&old, &new),
            self.rebase_animation_export(&old, &new),
            self.rebase_file_browser(&old, &new),
            self.rebase_monitor_color(&old, &new),
        ] {
            if let Err(e) = result {
                notices.push(e);
            }
        }
        self.asset_catalog = Default::default();
        self.resume_data_work_time();
        Ok(format!(
            "Moved {} files ({} bytes) to {}. Previous location retained at {}. Drawing pixels and Undo counts are preserved. Timelapse was checkpointed and stopped; reopen its local folder to resume. {}",
            out.files,
            out.bytes,
            new.display(),
            old.display(),
            notices.join("; ")
        ))
    }
    /// A data move freezes normal UI work, drains existing writers, then copies.
    /// Dispatched commands and local drops are replayed after the root is settled.
    pub(super) fn data_maintenance_gate(&mut self, ui: &mut egui::Ui) -> bool {
        let ctx = ui.ctx().clone();
        let mut p = std::mem::take(&mut self.data_maintenance);
        let had_job = p.job.is_some();
        if let Some(j) = &p.job {
            let result = match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err(
                    "Maintenance worker stopped; restart uses the migration journal".into(),
                )),
            };
            if let Some(result) = result {
                p.job = None;
                match result {
                    Ok(Value::Moved(out)) => {
                        p.message = self.moved_data_root(out).unwrap_or_else(|e| e)
                    }
                    Ok(Value::Restored) => {
                        if let Some(s) = self.storage.clone() {
                            self.load_materials(&s);
                        }
                        p.message="Brush tips and textures restored by their saved IDs; previous library bytes were retained. Existing drawing pixels and embedded stroke definitions are unchanged.".into();
                    }
                    Ok(Value::Recycled(path)) => {
                        p.message = format!(
                            "Moved {} to the OS Recycle Bin. Use system Restore to recover it. The active data root is unchanged.",
                            path.display()
                        )
                    }
                    Err(e) => {
                        self.resume_data_work_time();
                        p.message = e;
                    }
                }
            }
        }
        let mut action = None;
        let mut open = p.open;
        if p.open || p.busy() {
            egui::Window::new("Local data maintenance").open(&mut open).default_width(650.).show(&ctx,|ui|{
                p.controls.clear();
                ui.label(format!("Current location: {}",self.storage.as_ref().map_or_else(||"Unavailable".into(),|s|s.root().display().to_string())));
                ui.add_enabled_ui(!p.busy(),|ui|{
                    ui.label("Move to a new local folder (existing parent required):");
                    let r=ui.text_edit_singleline(&mut p.destination);p.controls.insert("destination",r.rect);
                    ui.horizontal(|ui|{
                        if ui.button("Choose parent folder…").clicked(){action=Some("parent");}
                        if ui.button("Return to previous location").clicked(){action=Some("previous");}
                    });
                    ui.checkbox(&mut p.reset_index,"Rebuild an unreadable asset index and keep its original bytes");
                    let r=ui.checkbox(&mut p.acknowledged,"Copy and verify before switching; keep the previous location");
                    p.controls.insert("acknowledge",r.rect);
                    let r=ui.add_enabled(p.acknowledged,egui::Button::new("Move data"));
                    p.controls.insert("move",r.rect);if r.clicked(){action=Some("move");}
                    ui.separator();
                    if ui.button("Rebuild / organize local asset catalog…").clicked(){action=Some("catalog");}
                    if ui.button("Manage local folders…").clicked(){action=Some("folders");}
                    if ui.button("Choose brush-tip / texture library backup…").clicked(){action=Some("backup");}
                    if let Some(path)=&p.backup{ui.label(path.display().to_string());if ui.button("Restore library; retain current bytes").clicked(){action=Some("restore");}}
                    ui.separator();
                    if let Some(storage)=&self.storage {
                        match core::retained(storage) {
                            Ok(paths)=>for path in paths {
                                let r=ui.selectable_label(p.cleanup.as_ref()==Some(&path),path.display().to_string());
                                if r.clicked(){p.cleanup=Some(path);p.confirm_cleanup=false;}
                            }
                            Err(e)=>{ui.label(e);}
                        }
                    }
                    ui.checkbox(&mut p.confirm_cleanup,"Confirm the selected retained folder and all its contents go to the OS Recycle Bin");
                    if ui.add_enabled(p.cleanup.is_some()&&p.confirm_cleanup,egui::Button::new("Recycle retained folder")).clicked(){action=Some("cleanup");}
                });
                if p.pending {ui.label("Waiting for current saves, indexing and file jobs. New commands are deferred.");}
                if let Some(j)=&p.job{
                    ui.label(format!("Data operation in progress; {} entries copied",j.progress.load(Ordering::Relaxed)));
                    let r=ui.button("Cancel and wait for recovery");p.controls.insert("cancel",r.rect);
                    if r.clicked(){j.cancel.store(true,Ordering::Relaxed);}
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
                if p.pending && ui.button("Cancel waiting").clicked(){p.pending=false;self.resume_data_work_time();}
                ui.label(&p.message);
            });
        }
        p.open = if p.busy() { true } else { open };
        let command_result = (|| -> Result<(), String> {
            match action {
                Some("parent") => {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        drawer_platform::file_browser::local_disk(&path)?;
                        p.destination = path.join("DrawerData").display().to_string();
                    }
                }
                Some("previous") => {
                    let path =
                        core::previous(self.storage.as_ref().ok_or("Local storage unavailable")?)?
                            .ok_or("No previous data move")?;
                    p.destination = path.display().to_string();
                }
                Some("move") => {
                    drawer_platform::file_browser::local_disk(
                        self.storage
                            .as_ref()
                            .ok_or("Local storage unavailable")?
                            .root(),
                    )?;
                    drawer_platform::file_browser::local_disk(Path::new(p.destination.trim()))?;
                    p.pending = true;
                    p.message = "Waiting for current writes before copying".into();
                }
                Some("catalog") => self.begin_asset_catalog()?,
                Some("folders") => self.begin_file_browser(super::file_browser::Mode::Open)?,
                Some("backup") => {
                    p.backup = rfd::FileDialog::new()
                        .add_filter("Drawer tip and texture library", &["json"])
                        .pick_file();
                }
                Some("restore") => {
                    let path = p.backup.clone().ok_or("Choose a backup")?;
                    drawer_platform::file_browser::local_disk(&path)?;
                    let storage = self.storage.clone().ok_or("Local storage unavailable")?;
                    p.job = Some(worker(move |_, _| {
                        core::restore_materials(&storage, &path)?;
                        Ok(Value::Restored)
                    })?);
                }
                Some("cleanup") => {
                    let path = p.cleanup.clone().ok_or("Choose a retained folder")?;
                    let s = self.storage.as_ref().ok_or("Local storage unavailable")?;
                    if !core::retained(s)?
                        .iter()
                        .any(|q| drawer_core::file_browser::same(q, &path))
                        || drawer_core::file_browser::remap(s.root(), &path, &path).is_some()
                        || self
                            .documents
                            .iter()
                            .flat_map(|d| {
                                d.path
                                    .iter()
                                    .chain(d.source_path.iter())
                                    .chain(d.scene.file_objects.entries.iter().map(|l| &l.path))
                            })
                            .chain(self.protected_sources.iter())
                            .chain(
                                self.references
                                    .session
                                    .workspace
                                    .entries
                                    .iter()
                                    .filter_map(|e| e.source.as_ref()),
                            )
                            .any(|q| drawer_core::file_browser::remap(q, &path, &path).is_some())
                    {
                        return Err("This folder is active or contains an open/protected source; it cannot be recycled".into());
                    }
                    drawer_platform::file_browser::local_disk(&path)?;
                    p.job = Some(worker(move |c, _| {
                        drawer_core::file_browser::reject_tree_links(&path, &c)?;
                        drawer_platform::file_browser::recycle(&path, c)?;
                        Ok(Value::Recycled(path))
                    })?);
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = command_result {
            p.message = e;
            p.pending = false;
        }
        let mut busy = p.busy();
        if busy || had_job {
            // Keep release/focus state current even while ordinary work is frozen.
            self.workspaces.input(&ctx);
            if !ctx.egui_wants_keyboard_input()
                && let Some((command, _, repeat)) = ctx.input_mut(|i| {
                    super::workspaces::shortcut(i, &self.workspaces.store.current.bindings)
                })
                && !repeat
                && !BrushTip::ALL.iter().any(|t| t.name() == command)
            {
                if p.commands.len() < 128 {
                    p.commands.push_back(command);
                } else {
                    p.message = "Deferred command limit128; wait for the data operation".into();
                }
            }
        }
        if busy {
            if ctx.input(|i| i.raw.dropped_files.len()) + p.drops.len() > 128 {
                p.message = "At most128 drops can wait; excess files were not opened".into();
            }
            p.drops.extend(ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .take(128usize.saturating_sub(p.drops.len()))
                    .map(|f| f.path().to_owned())
                    .collect::<Vec<_>>()
            }));
            ctx.input_mut(|i| {
                i.events.clear();
                i.raw.dropped_files.clear();
                i.pointer = Default::default();
                i.keys_down.clear();
            });
            if p.pending && p.job.is_none() && !self.quiesce_data_io(&ctx) {
                self.finish_stroke();
                self.settle_work_time();
                let prepared = (|| {
                    self.persist_workspace()?;
                    self.panels
                        .persist(self.storage.as_ref())
                        .map_err(|e| e.to_string())?;
                    self.freeze_data_work_time()?;
                    let storage = self.storage.clone().ok_or("Local storage unavailable")?;
                    let path = PathBuf::from(p.destination.trim());
                    let reset = p.reset_index;
                    worker(move |cancel, progress| {
                        let plan =
                            core::Plan::prepare_cancellable(&storage, &path, reset, &cancel)?;
                        plan.run(&cancel, &progress).map(Value::Moved)
                    })
                })();
                match prepared {
                    Ok(job) => {
                        p.job = Some(job);
                        p.pending = false;
                    }
                    Err(e) => {
                        self.resume_data_work_time();
                        p.pending = false;
                        p.message = e;
                    }
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
            busy = p.busy();
        }
        let command = if !busy { p.commands.pop_front() } else { None };
        let drops = if !busy {
            std::mem::take(&mut p.drops)
        } else {
            vec![]
        };
        self.data_maintenance = p;
        if let Some(command) = command {
            self.execute_command(&ctx, &command);
            ctx.request_repaint();
        }
        if !drops.is_empty()
            && let Err(e) = self.browser_drop(&ctx, drops)
        {
            self.status = Some(e);
        }
        busy || had_job
    }
}
pub(super) fn protected_storage_target(s: &UserStorage, path: &Path) -> bool {
    use drawer_core::file_browser::{remap, same};
    if remap(s.root(), path, path).is_some()
        || remap(path, s.root(), s.root()).is_some()
        || remap(s.anchor(), path, path).is_some()
    {
        return true;
    }
    let Some(parent) = s.anchor().parent() else {
        return true;
    };
    let Some(name) = s.anchor().file_name().and_then(|p| p.to_str()) else {
        return true;
    };
    [
        "location.json",
        "move.json",
        "location.json.write.tmp",
        "move.json.write.tmp",
    ]
    .iter()
    .any(|suffix| same(path, &parent.join(format!(".{name}.drawer-{suffix}"))))
}
#[path = "data_maintenance_verify.rs"]
mod verify;
pub(super) fn smoke(root: &Path) -> Result<(), String> {
    verify::run(root)
}
