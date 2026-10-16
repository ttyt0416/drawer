use super::*;
use drawer_core::{abr::Report, brush_presets::Book};
#[derive(Default)]
pub(super) struct Editor {
    path: String,
    name: String,
    report: Option<Report>,
    selected: Option<usize>,
    message: Option<String>,
}
impl Drawer {
    pub(super) fn stage_abr(
        &mut self,
        report: &Report,
        index: usize,
        name: &str,
        book: &mut Book,
    ) -> Result<(), String> {
        let (library, preset) = report
            .convert(index, name, &self.materials.library)
            .map_err(|e| e.to_string())?;
        let mut next = book.clone();
        next.put(preset, false)?;
        // Persist masks before publishing a settings draft that references them.
        self.publish_materials(library)?;
        *book = next;
        Ok(())
    }
    pub(super) fn abr_ui(&mut self, ui: &mut egui::Ui, e: &mut Editor, book: &mut Book) {
        ui.label("Inspect local ABR before converting a tip. Verified fixtures: v1/v2 sampled 8-bit raw, 1–512px. Original files are read-only; vendor-file compatibility is not qualified.");
        super::edit::text(ui, &mut e.path, "abr-import");
        if crate::help::button(ui, "Choose ABR file", "abr-import").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Brush file", &["abr"])
                .pick_file()
        {
            e.path = path.to_string_lossy().into();
        }
        if ui.button("Inspect ABR").clicked() {
            e.report = None;
            e.selected = None;
            e.message = None;
            match Report::read(PathBuf::from(&e.path).as_path()) {
                Ok(report) => e.report = Some(report),
                Err(err) => {
                    e.message = Some(format!(
                        "Inspection rejected: {err}. Previous presets/materials preserved."
                    ))
                }
            }
        }
        if let Some(report) = &e.report {
            ui.label(format!(
                "Header version {} · {} records",
                report.version,
                report.entries.len()
            ));
            for note in &report.notices {
                ui.colored_label(egui::Color32::YELLOW, note);
            }
            for (index, entry) in report.entries.iter().enumerate() {
                ui.push_id(index, |ui| {
                    if ui
                        .add_enabled(
                            entry.sample.is_some(),
                            egui::Button::selectable(
                                e.selected == Some(index),
                                format!(
                                    "{} · {} · type {} · {}",
                                    entry.number,
                                    entry.name,
                                    entry.kind,
                                    if entry.sample.is_some() {
                                        "tip convertible"
                                    } else {
                                        "not converted"
                                    }
                                ),
                            ),
                        )
                        .clicked()
                    {
                        e.selected = Some(index);
                        e.name = entry.name.clone();
                    }
                    for note in &entry.notes {
                        ui.label(note);
                    }
                });
            }
            ui.label("Unique preset / material name");
            super::edit::text(ui, &mut e.name, "abr-import");
            if ui
                .add_enabled(
                    e.selected.is_some() && self.storage.is_some(),
                    egui::Button::new("Convert selected tip into preset draft"),
                )
                .clicked()
            {
                e.message=Some(match self.stage_abr(report,e.selected.unwrap(),&e.name,book){Ok(())=>"Tip registered locally now. Preset is staged: Apply brush presets saves it; Cancel discards the preset draft. The registered mask remains available in local materials.".into(),Err(err)=>format!("Conversion rejected; previous presets/materials preserved: {err}")});
            }
        }
        if let Some(message) = &e.message {
            ui.label(message);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    let abr_root = root.join("abr-check");
    std::fs::create_dir(&abr_root).map_err(|e| e.to_string())?;
    let (lib, preset) = drawer_core::abr::verify(&abr_root).map_err(|e| e.to_string())?;
    let mut app = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(abr_root.join("user")).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    app.sync_tool_properties();
    let report =
        Report::read(&abr_root.join("independent-brush.abr")).map_err(|e| e.to_string())?;
    let mut book = Book::default();
    app.stage_abr(&report, 1, &preset.name, &mut book)?;
    if book.presets.len() != 1
        || *app.materials.library != lib
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("ABR conversion changed document or lost preset/material".into());
    }
    let prior = book.clone();
    let loaded = app.materials.library.clone();
    if app.stage_abr(&report, 1, &preset.name, &mut book).is_ok()
        || book != prior
        || *app.materials.library != *loaded
    {
        return Err("Duplicate ABR conversion changed stored values".into());
    }
    let mut next = app.capture_workspace();
    next.brushes = book.clone();
    app.stage_workspace(next, app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    let restart = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(abr_root.join("user")).map_err(|e| e.to_string())?);
    if restart.capture_workspace().brushes != book || *restart.materials.library != lib {
        return Err("Converted ABR preset/mask lost on restart".into());
    }
    let mut seq = drawer_core::input_replay::Sequence {
        background: [0; 4],
        ..Default::default()
    };
    seq.sequences[0] = preset.sequence;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let actual = super::input_replay::run_with_materials(&seq, loaded.clone(), &cancel, &progress)?;
    let fallback = super::input_replay::run(
        &drawer_core::input_replay::Sequence {
            background: [0; 4],
            ..Default::default()
        },
        &cancel,
        &progress,
    )?;
    if !actual.record.passed
        || actual.pixels == fallback.pixels
        || actual.pixels.iter().all(|p| p[3] == 0)
    {
        return Err("Converted ABR mask not used by drawing input".into());
    }
    seq.samples.pop();
    seq.expected_strokes = 0;
    let cancelled = super::input_replay::run_with_materials(&seq, loaded, &cancel, &progress)?;
    if !cancelled.record.passed || cancelled.pixels.iter().any(|p| p[3] != 0) {
        return Err("Cancelled ABR brush stroke changed pixels".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_conversion_restart_actual_input_and_failure_preservation() {
        let root = crate::tests::TestDirectory::new();
        smoke(&root.0).unwrap();
        let mut app = Drawer::new(&egui::Context::default(), None);
        let r = Report::read(&root.0.join("abr-check/independent-brush.abr")).unwrap();
        let mut book = Book::default();
        assert!(app.stage_abr(&r, 1, "No storage", &mut book).is_err());
        assert!(book.presets.is_empty());
        assert_eq!(
            *app.materials.library,
            drawer_core::material_library::Library::default()
        );
    }
}
