use super::*;
use drawer_core::{
    aids::Unit,
    canvas_size::{Dimensions, Preset, PresetBook},
};

pub(super) fn fields(
    ui: &mut egui::Ui,
    size: &mut Dimensions,
    edit_dpi: bool,
    dpi_keep_pixels: Option<bool>,
) -> bool {
    let mut changed = false;
    let mut unit = size.unit;
    egui::ComboBox::from_id_salt("size-unit")
        .selected_text(unit.label())
        .show_ui(ui, |ui| {
            for u in [Unit::Pixels, Unit::Millimeters, Unit::Inches] {
                ui.selectable_value(&mut unit, u, u.label());
            }
        });
    if unit != size.unit {
        let _ = size.switch_unit(unit);
        changed = true;
    }
    egui::Grid::new("size-values").show(ui, |ui| {
        for axis in 0..2 {
            ui.label(if axis == 0 { "Width" } else { "Height" });
            let response = ui.text_edit_singleline(if axis == 0 {
                &mut size.width
            } else {
                &mut size.height
            });
            if response.changed() {
                let _ = size.edited_axis(axis);
                changed = true;
            }
            ui.end_row();
        }
        if edit_dpi {
            ui.label("DPI");
            let previous = size.dpi.clone();
            if ui.text_edit_singleline(&mut size.dpi).changed() {
                if let Some(keep) = dpi_keep_pixels {
                    let _ = size.edited_dpi(&previous, keep);
                }
                changed = true;
            }
            ui.end_row();
        }
    });
    let mut locked = size.locked;
    if ui.checkbox(&mut locked, "Lock aspect ratio").changed() {
        let _ = size.lock(locked);
        changed = true;
    }
    if let (Ok([w, h]), Ok(dpi)) = (size.pixels(), drawer_core::canvas_size::dpi(&size.dpi)) {
        ui.label(format!(
            "{w} × {h} pixels · {:.3} × {:.3} mm",
            w as f64 * 25.4 / dpi,
            h as f64 * 25.4 / dpi
        ));
    }
    changed
}

#[derive(Default)]
pub(super) struct Presets {
    pub book: PresetBook,
    undo: Vec<PresetBook>,
    redo: Vec<PresetBook>,
    name: String,
    selected: Option<usize>,
    pub error: Option<String>,
}
impl Presets {
    pub fn load(&mut self, storage: &UserStorage) {
        match PresetBook::load(storage) {
            Ok(b) => self.book = b,
            Err(e) => self.error = Some(format!("Presets reset: {e}")),
        }
    }
    pub(super) fn apply_book(
        &mut self,
        next: PresetBook,
        storage: Option<&UserStorage>,
    ) -> Result<(), String> {
        if !next.valid() {
            return Err("Invalid canvas presets".into());
        }
        if next == self.book {
            if let Some(storage) = storage {
                next.save(storage).map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        if let Some(storage) = storage {
            next.save(storage).map_err(|e| e.to_string())?;
        }
        self.undo.push(std::mem::replace(&mut self.book, next));
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.selected = None;
        Ok(())
    }
    pub(super) fn add(
        &mut self,
        size: &Dimensions,
        name: String,
        storage: Option<&UserStorage>,
    ) -> Result<(), String> {
        if self.book.entries.iter().any(|p| p.name == name) {
            return Err(
                "That preset name exists. Choose another name or delete the old preset explicitly."
                    .into(),
            );
        }
        let mut next = self.book.clone();
        next.entries.push(Preset::from_dimensions(name, size)?);
        self.apply_book(next, storage)
    }
    pub(super) fn history(
        &mut self,
        redo: bool,
        storage: Option<&UserStorage>,
    ) -> Result<(), String> {
        let mut undo = self.undo.clone();
        let mut future = self.redo.clone();
        let next = if redo { future.pop() } else { undo.pop() };
        if let Some(next) = next {
            let current = self.book.clone();
            self.apply_book(next, storage)?;
            if redo {
                undo.push(current);
            } else {
                future.push(current);
            }
            self.undo = undo;
            self.redo = future;
        }
        Ok(())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        size: &mut Dimensions,
        storage: Option<&UserStorage>,
    ) -> bool {
        let mut changed = false;
        let mut action = None;
        ui.collapsing("Canvas presets", |ui| {
            egui::ComboBox::from_id_salt("canvas-presets")
                .selected_text(
                    self.selected
                        .and_then(|i| self.book.entries.get(i))
                        .map_or("Choose preset", |p| p.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    for (i, p) in self.book.entries.iter().enumerate() {
                        if ui
                            .selectable_value(&mut self.selected, Some(i), &p.name)
                            .clicked()
                        {
                            p.apply(size);
                            changed = true;
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.name);
                if ui.button("Save current size").clicked() {
                    action = Some(0);
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.selected.is_some(),
                        egui::Button::new("Delete selected preset"),
                    )
                    .clicked()
                {
                    action = Some(1);
                }
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo presets"))
                    .clicked()
                {
                    action = Some(2);
                }
                if ui
                    .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo presets"))
                    .clicked()
                {
                    action = Some(3);
                }
                if ui.button("Reset presets").clicked() {
                    action = Some(4);
                }
                if ui
                    .add_enabled(
                        storage.is_some(),
                        egui::Button::new("Restore previous presets"),
                    )
                    .clicked()
                {
                    action = Some(5);
                }
                if ui.button("Load presets…").clicked() {
                    action = Some(6);
                }
                if ui.button("Save presets…").clicked() {
                    action = Some(7);
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
        let result = match action {
            Some(0) => self.add(size, self.name.trim().to_string(), storage),
            Some(1) => {
                let mut next = self.book.clone();
                next.entries.remove(self.selected.unwrap());
                self.apply_book(next, storage)
            }
            Some(2) => self.history(false, storage),
            Some(3) => self.history(true, storage),
            Some(4) => self.apply_book(Default::default(), storage),
            Some(5) => PresetBook::read(
                &storage
                    .unwrap()
                    .root()
                    .join("settings/canvas-presets.previous.json"),
            )
            .map_err(|e| e.to_string())
            .and_then(|b| self.apply_book(b, storage)),
            Some(6) => rfd::FileDialog::new()
                .add_filter("Canvas presets", &["json"])
                .pick_file()
                .map_or(Ok(()), |p| {
                    PresetBook::read(&p)
                        .map_err(|e| e.to_string())
                        .and_then(|b| self.apply_book(b, storage))
                }),
            Some(7) => rfd::FileDialog::new()
                .add_filter("Canvas presets", &["json"])
                .set_file_name("canvas-presets.json")
                .save_file()
                .map_or(Ok(()), |p| self.book.export(&p).map_err(|e| e.to_string())),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.error = Some(error);
        } else if action.is_some() {
            self.error = None;
        }
        changed
    }
}
