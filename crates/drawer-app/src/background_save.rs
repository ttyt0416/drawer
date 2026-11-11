//! One writer per workspace. Every result belongs to the captured revision, never to later edits.
use super::*;
use std::thread::JoinHandle;
enum Job {
    Save {
        id: u64,
        checkpoint: drawer_core::saved_revision::Checkpoint,
        path: PathBuf,
        handle: JoinHandle<std::io::Result<drawer_core::atomic_file::Outcome>>,
    },
    Recovery {
        stamps: Vec<(u64, u64, bool)>,
        work: Option<String>,
        handle: JoinHandle<(Session, Result<(), String>)>,
    },
}
#[derive(Default)]
pub(crate) struct Background {
    job: Option<Job>,
    pub bytes: usize,
}
impl Background {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}
impl Drawer {
    pub(crate) fn queue_native_save(
        &mut self,
        n: usize,
        path: &Path,
        gate: Option<std::sync::mpsc::Receiver<()>>,
    ) -> Result<(), String> {
        if self.background.busy() {
            return Err(
                "A local save/recovery job is still writing; retry after it finishes".into(),
            );
        }
        let image = self.documents[n].allocation_bytes();
        let previous = match std::fs::metadata(path) {
            Ok(m) => usize::try_from(m.len()).map_err(|_| "Save predecessor size overflow")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e.to_string()),
        };
        let cost = image
            .saturating_mul(2)
            .max(image.saturating_add(previous.saturating_mul(2)))
            .saturating_add(2 * 1024 * 1024);
        let live = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        if live.saturating_add(cost) > self.resource_budget() {
            return Err(
                "Not enough memory for a frozen background save; use Save or free memory".into(),
            );
        }
        let checkpoint = self.documents[n].begin_save_checkpoint()?;
        let id = self.documents[n].id;
        let mut frozen = self.documents[n].current_copy();
        let target = path.to_owned();
        let handle = std::thread::Builder::new()
            .name("drawer-local-save".into())
            .spawn(move || {
                if let Some(gate) = gate {
                    gate.recv().map_err(std::io::Error::other)?;
                }
                drawer_core::files::save_report(&mut frozen, &target)
            });
        match handle {
            Ok(handle) => {
                self.background.bytes = cost;
                self.background.job = Some(Job::Save {
                    id,
                    checkpoint,
                    path: path.to_owned(),
                    handle,
                });
                self.status = Some(
                    "Saving captured revision in the background; later edits remain modified"
                        .into(),
                );
                Ok(())
            }
            Err(e) => {
                self.documents[n].cancel_save_checkpoint(checkpoint);
                Err(e.to_string())
            }
        }
    }
    pub(crate) fn start_recovery_job(&mut self) -> Result<(), String> {
        if self.background.busy() || self.drawing.pending() {
            return Err("Finish pending input/save before a recovery checkpoint".into());
        }
        self.recovery.last_success = false;
        let project = self.capture_recovery_project()?;
        let selected = self
            .documents
            .iter()
            .filter(|d| d.dirty || project.is_some())
            .collect::<Vec<_>>();
        let captured = selected.iter().map(|d| d.allocation_bytes()).sum::<usize>();
        let closed = project.as_ref().map_or(Ok(0usize), |(m, mapping)| {
            m.work
                .page_ids
                .iter()
                .zip(&m.work.pages)
                .filter(|(id, _)| !mapping.iter().any(|(p, _)| p == *id))
                .try_fold(0usize, |peak, (_, p)| {
                    drawer_platform::file_browser::local_disk(p)?;
                    let bytes = std::fs::metadata(p).map_err(|e| e.to_string())?.len();
                    let bytes = usize::try_from(bytes).map_err(|_| "Project page size overflow")?;
                    Ok::<_, String>(peak.max(bytes.saturating_mul(4)))
                })
        })?;
        let cost = captured
            .saturating_mul(if project.is_some() { 4 } else { 3 })
            .saturating_add(closed)
            .saturating_add(2 * 1024 * 1024);
        let live = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        if live.saturating_add(cost) > self.resource_budget() {
            return Err("Recovery capture exceeds the working budget; previous complete snapshots preserved".into());
        }
        let documents = selected
            .iter()
            .map(|d| d.current_copy())
            .collect::<Vec<_>>();
        let stamps = self
            .documents
            .iter()
            .map(|d| (d.id, d.revision, d.dirty))
            .collect();
        let work = self.recovery_project_stamp()?;
        let root = self
            .recovery
            .root
            .clone()
            .ok_or("Local scratch unavailable")?;
        let config = self.resources.current.clone();
        let mut session = self
            .recovery
            .session
            .take()
            .ok_or("Local recovery storage is unavailable")?;
        session.set_retention(config.recovery_keep as usize)?;
        let handle=std::thread::Builder::new().name("drawer-local-recovery".into()).spawn(move||{
            let result=(||->Result<(),String>{
                drawer_platform::file_browser::available_space(&root).and_then(|free|drawer_core::resource_settings::probe(&root,free))?;
                session.prune_generations()?;
                let used=drawer_core::resource_settings::recovery_usage(&root)?;let mut next=documents.iter().try_fold(65536u64,|n,d|drawer_core::files::estimate_native_bytes(d).map_err(|e|e.to_string()).and_then(|b|n.checked_add(b).ok_or("Recovery size overflow".into())))?;
                if let Some((m,mapping))=&project{for(id,p)in m.work.page_ids.iter().zip(&m.work.pages){let bytes=if let Some((_,doc))=mapping.iter().find(|(page,_)|page==id){let d=documents.iter().find(|d|d.id==*doc).ok_or("Open recovery page missing")?;drawer_core::files::estimate_native_bytes(d).map_err(|e|e.to_string())?}else{std::fs::metadata(p).map_err(|e|e.to_string())?.len()};next=next.checked_add(bytes).ok_or("Project recovery space overflow")?;}}
                let free=drawer_platform::file_browser::available_space(&root)?;if used.checked_add(next).is_none_or(|n|n>u64::from(config.recovery_mib)*1024*1024)||next>free{return Err("Recovery quota/disk space exceeded; previous complete copies preserved".into());}
                let refs=documents.iter().collect::<Vec<_>>();if let Some((model,mapping))=&project{session.checkpoint_project(&refs,model,mapping)}else{session.checkpoint(&refs)}
            })();(session,result)
        });
        match handle {
            Ok(handle) => {
                self.background.bytes = cost;
                self.background.job = Some(Job::Recovery {
                    stamps,
                    work,
                    handle,
                });
                Ok(())
            }
            Err(e) => {
                let root = self
                    .resources
                    .current
                    .root(self.storage.as_ref().ok_or("Storage unavailable")?)
                    .to_owned();
                self.init_recovery(&root);
                Err(format!(
                    "Recovery worker unavailable; previous session retained: {e}"
                ))
            }
        }
    }
    pub(crate) fn poll_background(&mut self, wait: bool) {
        let ready = self.background.job.as_ref().is_some_and(|j| {
            wait || match j {
                Job::Save { handle, .. } => handle.is_finished(),
                Job::Recovery { handle, .. } => handle.is_finished(),
            }
        });
        if !ready {
            return;
        }
        let job = self.background.job.take().unwrap();
        self.background.bytes = 0;
        match job {
            Job::Save {
                id,
                checkpoint,
                path,
                handle,
            } => {
                let result = handle
                    .join()
                    .map_err(|_| "Background writer stopped; previous file preserved".to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                if let Some(n) = self.documents.iter().position(|d| d.id == id) {
                    match result {
                        Ok(outcome) => match self.documents[n]
                            .complete_save_checkpoint(checkpoint, &path)
                        {
                            Ok(clean) => {
                                if clean {
                                    self.retire_recovery_document(id);
                                }
                                self.status = Some(if outcome.preserved_unverified.is_some() {
                                    "Saved captured revision; unreadable predecessor preserved separately".into()
                                } else if clean {
                                    "Background save complete".into()
                                } else {
                                    "Captured revision saved; later edits remain modified".into()
                                });
                                self.remember_file(&path);
                            }
                            Err(e) => self.status = Some(e),
                        },
                        Err(e) => {
                            self.documents[n].cancel_save_checkpoint(checkpoint);
                            self.status = Some(format!(
                                "Background save failed; current drawing and previous saved state preserved: {e}"
                            ));
                        }
                    }
                } else if let Err(e) = result {
                    self.status = Some(e);
                }
            }
            Job::Recovery {
                stamps,
                work,
                handle,
            } => match handle.join() {
                Ok((mut session, result)) => {
                    for id in std::mem::take(&mut self.recovery.retire) {
                        if let Err(e) = session.forget(id) {
                            self.recovery.error = Some(e);
                        }
                    }
                    let warning = session.take_maintenance_warning();
                    self.recovery.session = Some(session);
                    match result {
                        Ok(()) => {
                            self.recovery.last_success = true;
                            self.recovery.stamps = stamps;
                            self.recovery.work_stamp = work;
                            self.recovery.error = warning;
                        }
                        Err(e) => {
                            self.recovery.error = Some(e);
                            self.recovery.open = true;
                        }
                    }
                    self.recovery.last = Instant::now();
                }
                Err(_) => {
                    self.recovery.error = Some(
                        "Recovery worker stopped; previous complete snapshots retained".into(),
                    );
                    self.recovery.open = true;
                    if let Some(root) = self.recovery.root.clone() {
                        self.init_recovery(&root);
                    }
                }
            },
        }
    }
    pub(crate) fn drain_background(&mut self) {
        self.poll_background(true);
    }
}

#[path = "background_save_verify.rs"]
pub(crate) mod verify;
