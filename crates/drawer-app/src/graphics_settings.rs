use crate::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    version: u32,
    pub software: bool,
    pub last_failure: Option<String>,
    pub last_gpu: Option<[String; 3]>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            software: false,
            last_failure: None,
            last_gpu: None,
        }
    }
}
impl Settings {
    pub fn read(storage: &UserStorage) -> Result<Self, String> {
        let path = storage.root().join("settings/graphics.json");
        let path = drawer_core::file_browser::new_path(&path)?;
        if !path.exists() {
            return Ok(Self::default());
        }
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4096 {
            return Err("Graphics preference exceeds4KiB; preserved".into());
        }
        let value: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if value.version != 1
            || value.last_failure.as_ref().is_some_and(|s| s.len() > 1024)
            || value
                .last_gpu
                .as_ref()
                .is_some_and(|v| v.iter().any(|s| s.len() > 512))
        {
            return Err("Unsupported graphics preferences; preserved".into());
        }
        Ok(value)
    }
    pub fn save(&self, storage: &UserStorage) -> Result<(), String> {
        let path = storage.root().join("settings/graphics.json");
        drawer_core::settings_compat::guard(&path, self).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        drawer_core::atomic_file::publish(
            &path,
            &path.with_file_name("graphics-write.tmp"),
            |w| {
                use std::io::Write;
                w.write_all(&bytes)
            },
            |p| {
                let v: Self =
                    serde_json::from_slice(&std::fs::read(p)?).map_err(std::io::Error::other)?;
                if v.version != 1 {
                    return Err(std::io::Error::other("Unsupported graphics version"));
                }
                Ok(())
            },
            &mut |_| Ok(()),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub settings: Settings,
    pub open: bool,
    pub running_cpu: bool,
    pub request_cpu: bool,
}
impl Drawer {
    pub(super) fn load_graphics(&mut self, storage: &UserStorage) {
        match Settings::read(storage) {
            Ok(s) => self.graphics.settings = s,
            Err(e) => self.status = Some(e),
        }
    }
    pub(super) fn graphics_window(&mut self, ctx: &egui::Context) {
        if !self.graphics.open {
            return;
        }
        let mut open = true;
        let mut software = self.graphics.settings.software;
        let mut apply = false;
        egui::Window::new(crate::i18n::text("Display recovery")).open(&mut open).default_width(430.).show(ctx,|ui|{
            ui.label(if self.graphics.running_cpu{"Current display: local CPU/software"}else{"Current display: GPU/OpenGL; document composition is local CPU"});
            ui.checkbox(&mut software,"Use software display after restart");
            if !self.graphics.running_cpu&&ui.button("Switch this window to software display").clicked(){software=true;apply=true;}
            if self.graphics.running_cpu{ui.label("To try the GPU again, clear this option and restart. Open documents and Undo remain local.");}
            if let Some(reason)=&self.graphics.settings.last_failure{ui.label(format!("Last display failure: {reason}"));}
        });
        self.graphics.open = open;
        if software != self.graphics.settings.software || apply {
            let mut next = self.graphics.settings.clone();
            next.software = software;
            let result = self.storage.as_ref().map_or(Ok(()), |s| next.save(s));
            match result {
                Ok(()) => {
                    self.graphics.settings = next;
                    self.graphics.request_cpu = apply;
                }
                Err(e) => self.status = Some(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn software_preference_restart_and_future_version_preservation() {
        let root = crate::tests::TestDirectory::new();
        let storage = UserStorage::at(root.0.join("graphics-profile")).unwrap();
        let mut settings = Settings::read(&storage).unwrap();
        assert!(!settings.software);
        settings.software = true;
        settings.save(&storage).unwrap();
        assert!(Settings::read(&storage).unwrap().software);
        let path = storage.root().join("settings/graphics.json");
        let future = br#"{"version":999,"software":false,"last_failure":null,"last_gpu":null}"#;
        std::fs::write(&path, future).unwrap();
        assert!(Settings::read(&storage).is_err());
        assert!(settings.save(&storage).is_err());
        assert_eq!(std::fs::read(path).unwrap(), future);
    }
}
