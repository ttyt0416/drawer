use super::*;
#[derive(Default)]
pub(super) struct Health {
    pub message: Option<String>,
    pub checked: Option<std::time::Instant>,
    pub free: Option<u64>,
    pub recovery: u64,
}
impl Drawer {
    pub(super) fn check_resource_health(&mut self) {
        let result = (|| -> Result<(u64, u64), String> {
            let storage = self
                .storage
                .as_ref()
                .ok_or("Local settings storage unavailable")?;
            let root = self.resources.current.root(storage);
            let free = drawer_platform::file_browser::available_space(root)?;
            drawer_core::resource_settings::probe(root, free)?;
            Ok((free, drawer_core::resource_settings::recovery_usage(root)?))
        })();
        self.resources.health.checked = Some(std::time::Instant::now());
        match result {
            Ok((free, used)) => {
                self.resources.health.free = Some(free);
                self.resources.health.recovery = used;
                self.resources.health.message = if free < 16 * 1024 * 1024 {
                    Some("Scratch disk is almost full. Recovery may stop; save to a local drive with enough space.".into())
                } else if used
                    >= u64::from(self.resources.current.recovery_mib) * 1024 * 1024 * 9 / 10
                {
                    Some("Recovery storage is near its quota. Review unresolved snapshots; they are never deleted to make room.".into())
                } else {
                    None
                };
            }
            Err(e) => {
                self.resources.health.free = None;
                self.resources.health.message = Some(format!(
                    "Scratch cannot be written: {e}. Previous drawings/recovery copies are preserved. Choose a working local path or save elsewhere."
                ));
            }
        }
    }
    pub(super) fn resource_health_banner(&self, ui: &mut egui::Ui) {
        let used = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        let limit = self.resource_budget();
        if used >= limit * 4 / 5 {
            ui.colored_label(egui::Color32::YELLOW,format!("Drawing/history memory {used} / {limit} bytes. {}",if used>=limit{"New edits stop until memory is freed. Save/close drawings, clear history or raise the budget."}else{"Close saved drawings or reduce retained history if needed."}));
        }
        if let Some(message) = &self.resources.health.message {
            ui.colored_label(egui::Color32::YELLOW, message);
        }
    }
}
pub(super) fn denied(root: &std::path::Path) -> Result<(), String> {
    let free = drawer_platform::file_browser::available_space(root)?;
    if free < 1024 * 1024 {
        return Err("Permission test requires actual free space".into());
    }
    match drawer_core::resource_settings::probe(root, free) {
        Err(e) if e.starts_with("Scratch is not writable:") => Ok(()),
        Err(e) => Err(format!("Unexpected permission-test failure: {e}")),
        Ok(()) => Err("Write permission was not revoked".into()),
    }
}
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    drawer_core::resource_failure_verify::run()?;
    let dir = root.join("resource-failure-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let ctx = egui::Context::default();
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let mut app = Drawer::new(&ctx, None).with_storage(storage);
        app.create_canvas(CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let source = dir.join("normal.drawer");
        app.save_to(0, &source)?;
        let normal = std::fs::read(&source).map_err(|e| e.to_string())?;
        app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let state = app.documents[0].snapshot();
        let revision = app.documents[0].revision;
        if app.save_to_space(0, &source, Some(0)).is_ok() {
            return Err("Full disk was accepted".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&source)
                .map_err(|e| e.to_string())?;
            if app.save_to(0, &source).is_ok() {
                return Err("Locked destination was replaced".into());
            }
            drop(lock);
        }
        let scratch = dir.join("scratch");
        std::fs::create_dir(&scratch).map_err(|e| e.to_string())?;
        app.resources.current.scratch = Some(scratch.clone());
        app.init_recovery(&scratch);
        app.checkpoint_recovery()?;
        app.quiesce_recovery();
        let detached = dir.join("detached-scratch");
        std::fs::rename(&scratch, &detached).map_err(|e| e.to_string())?;
        app.check_resource_health();
        if app.resources.health.message.is_none()
            || app.checkpoint_recovery().is_ok()
            || app.documents[0].snapshot() != state
            || app.documents[0].revision != revision
            || !app.documents[0].dirty
            || std::fs::read(&source).map_err(|e| e.to_string())? != normal
        {
            return Err("Detached scratch changed normal state or lacked warning".into());
        }
        let copies = drawer_core::recovery::scan(&detached)?;
        if copies.len() != 1 || copies[0].open()?.snapshot() != state {
            return Err("Detached scratch lost the previous complete recovery".into());
        }
        std::fs::rename(&detached, &scratch).map_err(|e| e.to_string())?;
        app.resume_recovery();
        app.check_resource_health();
        if app.resources.health.message.is_some() {
            return Err("Scratch did not recover after reattachment".into());
        }
        app.save_to(0, &source)?;
        if app.documents[0].dirty
            || drawer_core::files::open(&source, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                != state
        {
            return Err("Retry did not preserve and save drawing".into());
        }
        app.finish_recovery();
        drop(app);
        Ok(())
    })();
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn full_disk_locked_file_detached_scratch_and_retry() {
        let root =
            std::env::temp_dir().join(format!("drawer-resource-failure-ui-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let r = super::run(&root);
        std::fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
