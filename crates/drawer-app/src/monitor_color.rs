use super::*;
use drawer_core::{monitor_color::Settings, soft_proof::Proof};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Default)]
pub(super) struct Panel {
    pub(super) saved: Settings,
    draft: Settings,
    pub(super) stages: [Option<Arc<Proof>>; 2],
    pub(super) serial: u64,
    open: bool,
    undo: Vec<Settings>,
    message: String,
    controls: BTreeMap<&'static str, egui::Rect>,
}
impl Drawer {
    pub(super) fn load_monitor_color(&mut self, storage: &UserStorage) {
        match Settings::load(storage) {
            Ok(s) => {
                self.monitor_color.draft = s.clone();
                self.monitor_color.saved = s.clone();
                match s.prepare() {
                    Ok(stages) => {
                        self.monitor_color.stages = stages;
                        self.monitor_color.serial = 1;
                    }
                    Err(e) => {
                        self.monitor_color.message =
                            format!("Monitor ICC unavailable; sRGB display assumed: {e}")
                    }
                }
            }
            Err(e) => {
                self.monitor_color.message =
                    format!("Monitor settings unreadable; sRGB display assumed: {e}")
            }
        }
        if !self.monitor_color.message.is_empty() {
            self.status = Some(self.monitor_color.message.clone());
        }
    }
    pub(super) fn begin_monitor_color(&mut self) {
        self.monitor_color.draft = self.monitor_color.saved.clone();
        self.monitor_color.open = true;
    }
    fn save_monitor_color(&self, p: &mut Panel, s: Settings, record: bool) -> Result<(), String> {
        let stages = s.prepare()?;
        s.save(
            self.storage
                .as_ref()
                .ok_or("Local settings folder unavailable")?,
        )?;
        if record {
            p.undo.push(p.saved.clone());
            if p.undo.len() > 20 {
                p.undo.remove(0);
            }
        }
        p.stages = stages;
        p.serial = p.serial.wrapping_add(1).max(1);
        p.saved = s.clone();
        p.draft = s;
        p.message = "Monitor settings saved; drawing pixels and output profiles unchanged".into();
        Ok(())
    }
    pub(super) fn monitor_color_window(&mut self, ctx: &egui::Context) {
        let mut p = std::mem::take(&mut self.monitor_color);
        if !p.open {
            self.monitor_color = p;
            return;
        }
        let mut open = true;
        let mut command = None;
        p.controls.clear();
        egui::Window::new("Color management — separate profile roles").default_pos(egui::pos2(25.,25.)).default_width(650.).open(&mut open).show(ctx,|ui|{
            ui.label(format!("Document working profile: {}",self.active.map_or("No document",|n|self.documents[n].color.profile.label())));
            ui.label("The working profile and channel values belong to the document. Monitor ICC is application-local and applied only after output proof on canvas views. Missing profiles assume sRGB; no OS monitor profile is guessed.");
            ui.horizontal(|ui|{if ui.button("Document working profile…").clicked(){command=Some("document");}
if ui.button("Output soft proof / rendering intent…").clicked(){command=Some("proof");}});
            for n in 0..2{ui.separator();ui.label(if n==0{"Main and tiled canvas monitor"}else{"Separate display monitor"});let mut text=p.draft.profiles[n].as_ref().map_or(String::new(),|v|v.display().to_string());if ui.text_edit_singleline(&mut text).changed(){p.draft.profiles[n]=(!text.trim().is_empty()).then(||PathBuf::from(text.trim()));}ui.horizontal(|ui|{if ui.button(format!("Choose monitor {} RGB ICC…",n+1)).clicked()&&let Some(path)=rfd::FileDialog::new().add_filter("RGB ICC",&["icc","icm"]).pick_file(){p.draft.profiles[n]=Some(path);}let r=ui.button("Assume sRGB");p.controls.insert(if n==0{"clear-main"}else{"clear-secondary"},r.rect);if r.clicked(){p.draft.profiles[n]=None;}});}
            ui.small("Monitor transform: Relative Colorimetric, sRGB → monitor RGB. Output proof has its own profile and four rendering intents. Exports use document/output conversion, never monitor calibration.");
            ui.horizontal(|ui|{for(key,label)in[("save","Validate and save / reload"),("undo","Undo monitor settings"),("cancel","Cancel draft")]{let r=ui.button(label);p.controls.insert(key,r.rect);if r.clicked(){command=Some(key);}}});ui.label(&p.message);
        });
        let result = match command {
            Some("save") => {
                let s = p.draft.clone();
                self.save_monitor_color(&mut p, s, true)
            }
            Some("undo") => {
                if let Some(s) = p.undo.last().cloned() {
                    let r = self.save_monitor_color(&mut p, s, false);
                    if r.is_ok() {
                        p.undo.pop();
                    }
                    r
                } else {
                    Err("No monitor settings to restore".into())
                }
            }
            Some("document") => self.begin_document_color(),
            Some("proof") => {
                self.open_proof();
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            p.message = e;
        }
        if !open || command == Some("cancel") || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            p.open = false;
            p.draft = p.saved.clone();
        }
        self.monitor_color = p;
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("monitor-color");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let ctx = egui::Context::default();
        let data = dir.join("user");
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(data.clone()).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::png_io_verify::fixture()?);
        app.documents[0].layers[0].pixels.fill([220, 30, 80, 128]);
        let path = dir.join("monitor.icc");
        std::fs::write(
            &path,
            drawer_core::color_precision_verify::display_p3_fixture(),
        )
        .map_err(|e| e.to_string())?;
        let original = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let png = dir.join("before.png");
        drawer_core::png_export::export(&app.documents[0], &png, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&png).map_err(|e| e.to_string())?;
        app.execute_command(&ctx, "color-management");
        app.monitor_color.draft.profiles[0] = Some(path.clone());
        let mut time = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, app, *t, events);
        };
        let click = |app: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
            let r = app
                .monitor_color
                .controls
                .get(key)
                .copied()
                .ok_or("Missing monitor control")?;
            for pressed in [true, false] {
                frame(
                    app,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            for _ in 0..3 {
                frame(app, t, vec![]);
            }
            Ok(())
        };
        click(&mut app, &mut time, "save")?;
        if app.monitor_color.saved.profiles[0] != Some(path.clone()) {
            return Err(format!(
                "Monitor pointer save failed: {}",
                app.monitor_color.message
            ));
        }
        let raw = app
            .canvas_view
            .preview_sample
            .ok_or("No monitor canvas sample")?;
        if raw.0 == raw.1 {
            return Err("Monitor stage did not change canvas output".into());
        }
        if app
            .display
            .canvas
            .preview_sample
            .is_some_and(|(a, b)| a != b)
        {
            return Err("Main monitor changed the secondary view".into());
        }
        let after = dir.join("after.png");
        drawer_core::png_export::export(&app.documents[0], &after, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
        if std::fs::read(after).map_err(|e| e.to_string())? != bytes
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != history
        {
            return Err("Monitor settings changed export or document history".into());
        }
        app.monitor_color.draft.profiles[0] = Some(dir.join("missing.icc"));
        click(&mut app, &mut time, "save")?;
        if app.monitor_color.saved.profiles[0] != Some(path.clone()) {
            return Err("Missing ICC changed monitor settings".into());
        }
        click(&mut app, &mut time, "cancel")?;
        let restart =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(data).map_err(|e| e.to_string())?);
        if restart.monitor_color.saved.profiles[0] != Some(path) {
            return Err("Monitor preferences did not restart".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}

impl Drawer {
    pub(super) fn rebase_monitor_color(
        &mut self,
        old: &std::path::Path,
        new: &std::path::Path,
    ) -> Result<(), String> {
        let p = &mut self.monitor_color;
        for s in std::iter::once(&mut p.draft)
            .chain(std::iter::once(&mut p.saved))
            .chain(&mut p.undo)
        {
            *s = drawer_core::storage_migration::rebase_settings(s, old, new)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn profile_roles_pointer_monitor_persistence_cancel_and_export_isolation() {
        let t = crate::tests::TestDirectory::new();
        super::smoke(&t.0).unwrap();
    }
}
