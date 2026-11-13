use super::*;
impl Drawer {
    pub(super) fn registered_model_config(&self, task: Task) -> Result<Config, String> {
        let mut config = Config {
            task,
            page: 1,
            ..Default::default()
        };
        if let Some(storage) = &self.storage
            && let Some(r) = drawer_infer::bundle::registered(storage, task)?
        {
            config.manifest = r.path(storage).to_string_lossy().into_owned();
            config.approved_manifest = Some(r.manifest_sha256.clone());
            config.model_label = format!("{} {} · {}", r.name, r.version, r.license);
        }
        if let Some(storage) = &self.storage {
            config.compute = crate::local_compute::Settings::load(storage).map_err(|e| {
                format!("Local processing preference could not be read; original preserved: {e}")
            })?;
        }
        Ok(config)
    }
    fn model_memory_admission(&self, path: &Path) -> Result<(), String> {
        let cost = drawer_infer::bundle::review_memory(path)?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(cost) > self.resource_budget() {
            return Err("Insufficient workspace memory to inspect/copy this model; increase the local RAM limit or free document/history memory".into());
        }
        Ok(())
    }
    pub(super) fn review_model_bundle(&mut self) -> Result<(), String> {
        let draft = self
            .local_inference
            .draft
            .as_ref()
            .ok_or("Open local models first")?;
        let path = PathBuf::from(&draft.config.manifest);
        let task = draft.config.task;
        self.model_memory_admission(&path)?;
        let review = drawer_infer::bundle::review(&path, task)?;
        let draft = self
            .local_inference
            .draft
            .as_mut()
            .ok_or("Local models closed")?;
        draft.model_review = Some(review);
        draft.allow_model_copy = false;
        Ok(())
    }
    pub(super) fn install_model_bundle(&mut self) -> Result<(), String> {
        let draft = self
            .local_inference
            .draft
            .as_ref()
            .ok_or("Open local models first")?;
        let hash = draft
            .model_review
            .as_ref()
            .ok_or("Inspect the local model license first")?
            .manifest_sha256
            .clone();
        let path = PathBuf::from(&draft.config.manifest);
        let task = draft.config.task;
        let permission = draft.allow_model_copy;
        if !permission {
            return Err(
                "Confirm that you reviewed LICENSE.txt and may copy/use this model locally".into(),
            );
        }
        self.model_memory_admission(&path)?;
        let storage = self
            .storage
            .clone()
            .ok_or("Local storage unavailable; choose a local data folder first")?;
        let available = drawer_platform::file_browser::available_space(storage.root())?;
        let registration =
            drawer_infer::bundle::install(&storage, &path, task, &hash, permission, available)?;
        let draft = self
            .local_inference
            .draft
            .as_mut()
            .ok_or("Local models closed")?;
        draft.config.manifest = registration.path(&storage).to_string_lossy().into_owned();
        draft.config.approved_manifest = Some(registration.manifest_sha256);
        draft.scroll_to_input = true;
        draft.config.model_label = format!(
            "{} {} · {}",
            registration.name, registration.version, registration.license
        );
        draft.model_review = None;
        draft.allow_model_copy = false;
        draft.input = None;
        draft.images = None;
        draft.prepared = None;
        draft.result = None;
        Ok(())
    }
    pub(super) fn forget_model_bundle(&mut self) -> Result<(), String> {
        let draft = self
            .local_inference
            .draft
            .as_ref()
            .ok_or("Open local models first")?;
        let storage = self.storage.as_ref().ok_or("Local storage unavailable")?;
        drawer_infer::bundle::forget(storage, draft.config.task)?;
        let draft = self
            .local_inference
            .draft
            .as_mut()
            .ok_or("Local models closed")?;
        draft.config.approved_manifest = None;
        draft.config.model_label.clear();
        draft.model_review = None;
        draft.allow_model_copy = false;
        draft.input = None;
        draft.images = None;
        draft.prepared = None;
        draft.result = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consent_memory_restart_and_forget_preserve_document_and_cached_bytes() {
        let t = crate::tests::TestDirectory::new();
        let ctx = egui::Context::default();
        let storage = UserStorage::at(t.0.join("user")).unwrap();
        let mut app = Drawer::new(&ctx, None).with_storage(storage.clone());
        app.create_canvas(crate::tests::tiny_spec()).unwrap();
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let source = drawer_infer::fixtures::bundle(&t.0.join("source"), Task::Body).unwrap();
        app.begin_local_inference().unwrap();
        app.local_inference.draft.as_mut().unwrap().config.manifest = source.display().to_string();
        assert!(app.start_inference(&ctx, true).is_err());
        assert!(app.local_inference.job.is_none());
        let weights = source.parent().unwrap().join("weights.onnx");
        let original = std::fs::read(&weights).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&weights)
            .unwrap()
            .set_len(33 * 1024 * 1024)
            .unwrap();
        app.resources.current.ram_mib = 64;
        assert!(app.review_model_bundle().unwrap_err().contains("memory"));
        assert!(
            app.start_inference(&ctx, false)
                .unwrap_err()
                .contains("memory")
        );
        assert!(!storage.root().join("models").exists());
        std::fs::write(&weights, &original).unwrap();
        app.resources.current.ram_mib = 256;
        app.review_model_bundle().unwrap();
        assert!(app.install_model_bundle().is_err());
        app.local_inference.draft.as_mut().unwrap().allow_model_copy = true;
        app.install_model_bundle().unwrap();
        let cached = PathBuf::from(&app.local_inference.draft.as_ref().unwrap().config.manifest);
        let cached_bytes = std::fs::read(&cached).unwrap();
        assert!(cached.starts_with(storage.root().join("models")));
        app.local_inference.draft = None;
        app.begin_local_inference().unwrap();
        assert!(
            app.local_inference
                .draft
                .as_ref()
                .unwrap()
                .config
                .approved_manifest
                .is_some()
        );
        app.forget_model_bundle().unwrap();
        assert!(
            app.local_inference
                .draft
                .as_ref()
                .unwrap()
                .config
                .approved_manifest
                .is_none()
        );
        assert_eq!(std::fs::read(cached).unwrap(), cached_bytes);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), counts);
    }
}
