use super::*;
use drawer_core::resource_settings::{self as core, Settings};
#[derive(Default)]
pub(super) struct Panel {
    pub health: resource_health::Health,
    pub open: bool,
    pub current: Settings,
    draft: Settings,
    pending: Option<Settings>,
    pub error: Option<String>,
}
impl Drawer {
    pub(super) fn resource_budget(&self) -> usize {
        self.resources.current.budget()
    }
    pub(super) fn load_resources(&mut self, storage: &UserStorage) {
        match Settings::load(storage) {
            Ok(s) => {
                self.resources.current = s.clone();
                self.resources.draft = s;
            }
            Err(e) => {
                self.resources.error = Some(format!(
                    "Resource settings could not be read; original file preserved: {e}"
                ))
            }
        }
    }
    fn apply_resources(&mut self, mut next: Settings) -> Result<(), String> {
        next.valid()?;
        let storage = self
            .storage
            .as_ref()
            .ok_or("Local settings storage unavailable")?
            .clone();
        if self.documents.iter().any(|d| d.dirty)
            || self.drawing.pending()
            || self.file_browser.mutating()
        {
            return Err(
                "Resource changes wait until drawings are saved and pending work finishes".into(),
            );
        }
        if self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>()
            > next.budget()
            || self
                .documents
                .iter()
                .any(|d| d.width > next.max_dimension || d.height > next.max_dimension)
        {
            return Err("Open documents exceed the proposed limits; close them or choose larger limits. Existing settings preserved".into());
        }
        for d in &self.documents {
            d.can_configure_history(next.history)?;
        }
        let root = next.root(&storage).to_owned();
        let free = drawer_platform::file_browser::available_space(&root)?;
        core::probe(&root, free)?;
        let old = self.resources.current.root(&storage).to_owned();
        if root != old && !next.retained.contains(&old) {
            next.retained.push(old);
        }
        next.valid()?;
        let session = drawer_core::recovery::Session::create(&root)?;
        if let Err(e) = next.save(&storage) {
            let _ = session.finish();
            return Err(e.to_string());
        }
        self.install_resource_recovery(&root, session);
        self.resources.current = next.clone();
        self.resources.draft = next;
        self.resources.error = None;
        self.status = Some(
            "Local resource settings saved. Unresolved recovery copies remain until your decision."
                .into(),
        );
        Ok(())
    }
    pub(super) fn resource_settings_window(&mut self, ctx: &egui::Context) {
        if self.resources.pending.is_some()
            && self.documents.iter().all(|d| !d.dirty)
            && !self.drawing.pending()
            && !ctx.input(|i| i.pointer.any_down())
            && !self.file_browser.mutating()
        {
            let next = self.resources.pending.take().unwrap();
            if let Err(e) = self.apply_resources(next) {
                self.resources.error = Some(e);
            }
        }
        if !self.resources.open {
            return;
        }
        if self
            .resources
            .health
            .checked
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(10))
            && !self.drawing.pending()
            && !ctx.input(|i| i.pointer.any_down())
        {
            self.check_resource_health();
        }
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Local resources").open(&mut open).default_width(470.).show(ctx,|ui|{
            if ui.button("Check scratch permissions and free space now").clicked(){self.check_resource_health();}
            if let Some(free)=self.resources.health.free{ui.small(format!("Scratch free {free} bytes; recovery {} bytes / {}MiB",self.resources.health.recovery,self.resources.current.recovery_mib));}
            self.resource_health_banner(ui);
            ui.label("Limits apply to known drawing buffers, not the entire process. Changes wait until all drawings are saved and input/file work is idle.");
            ui.horizontal(|ui|{ui.label("Working memory MiB (64–256)");ui.add(egui::DragValue::new(&mut self.resources.draft.ram_mib).range(64..=256));});
            ui.horizontal(|ui|{ui.label("Maximum canvas dimension (1–32768)");ui.add(egui::DragValue::new(&mut self.resources.draft.max_dimension).range(1..=32768));});
            ui.horizontal(|ui|{ui.label("Recovery storage MiB (16–4096)");ui.add(egui::DragValue::new(&mut self.resources.draft.recovery_mib).range(16..=4096));});
            ui.horizontal(|ui|{ui.label("Undo steps (1–1000)");ui.add(egui::DragValue::new(&mut self.resources.draft.history.steps).range(1..=1000));});
            ui.horizontal(|ui|{ui.label("History MiB (1–64)");ui.add(egui::DragValue::new(&mut self.resources.draft.history.memory_mib).range(1..=64));});
            ui.horizontal(|ui|{ui.label("Explicit command group interval ms (0–5000;0 disables)");ui.add(egui::DragValue::new(&mut self.resources.draft.history.group_ms).range(0..=5000));});
            ui.small("Only consecutive updates of one explicit command may merge. Separate strokes, layer changes and document switches remain separate. Oldest Undo records are discarded; Redo alone above a new limit prevents applying it.");
            ui.label(format!("Scratch: {}",self.resources.draft.scratch.as_deref().map_or("Application data".into(),|p|p.display().to_string())));
            if ui.button("Choose local scratch folder…").clicked()&&let Some(p)=rfd::FileDialog::new().pick_folder(){self.resources.draft.scratch=Some(p);}
            if ui.button("Use application data folder").clicked(){self.resources.draft.scratch=None;}
            if self.resources.pending.is_some(){ui.label("Pending: save/close modified drawings to apply.");if ui.button("Cancel pending changes").clicked(){self.resources.pending=None;}}
            if let Some(e)=&self.resources.error{ui.colored_label(egui::Color32::LIGHT_RED,e);}
            apply=ui.button("Validate and apply when idle").clicked();if ui.button("Revert draft").clicked(){self.resources.draft=self.resources.current.clone();}
        });
        self.resources.open = open;
        if apply {
            match self.resources.draft.valid() {
                Ok(()) => self.resources.pending = Some(self.resources.draft.clone()),
                Err(e) => self.resources.error = Some(e),
            }
        }
    }
}
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let dir = root.join("resource-settings-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        core::verify::run(&dir)?;
        let storage = UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None).with_storage(storage.clone());
        app.create_canvas(CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        })
        .map_err(|e| e.to_string())?;
        let scratch = dir.join("scratch");
        std::fs::create_dir(&scratch).map_err(|e| e.to_string())?;
        let mut next = app.resources.current.clone();
        next.scratch = Some(scratch.clone());
        next.ram_mib = 64;
        next.max_dimension = 64;
        next.recovery_mib = 16;
        if app.apply_resources(next.clone()).is_ok() || app.resources.current.scratch.is_some() {
            return Err("Dirty document changed scratch immediately".into());
        }
        app.resources.pending = Some(next);
        input_replay::frame(&ctx, &mut app, 0.1, vec![]);
        if app.resources.pending.is_none() {
            return Err("Pending scratch applied during edit".into());
        }
        app.documents[0].mark_saved(dir.join("logical.drawer"));
        input_replay::frame(&ctx, &mut app, 0.2, vec![]);
        if app.resources.current.scratch.as_ref() != Some(&scratch)
            || Settings::load(&storage).map_err(|e| e.to_string())? != app.resources.current
        {
            return Err(format!(
                "Idle resource apply/restart failed: {:?}",
                app.resources.error
            ));
        }
        if app
            .create_canvas(CanvasSpec {
                width: 65,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            })
            .is_ok()
            || app.documents.len() != 1
        {
            return Err("Configured dimension cap ignored".into());
        }
        let before = app.resources.current.clone();
        let mut bad = before.clone();
        bad.scratch = Some(dir.join("missing"));
        if app.apply_resources(bad).is_ok() || app.resources.current != before {
            return Err("Failed path move changed settings".into());
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
    fn idle_apply_restart_limits_and_failed_move() {
        let root = std::env::temp_dir().join(format!(
            "drawer-resource-settings-ui-{}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let r = super::run(&root);
        std::fs::remove_dir_all(root).unwrap();
        r.unwrap();
    }
}
