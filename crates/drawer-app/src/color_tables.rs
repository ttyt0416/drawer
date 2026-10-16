use super::*;
use drawer_core::color_tables::History;
#[derive(Default)]
pub(super) struct Tables {
    pub history: History,
    recent_restore: Option<Vec<[u8; 4]>>,
    selected: Option<u64>,
    name: String,
}
impl Drawer {
    pub(super) fn register_sample(&mut self, table: Option<u64>, color: [u8; 4]) {
        let before = self.workspaces.store.current.color_tables.clone();
        let result = self
            .workspaces
            .store
            .current
            .color_tables
            .add_to(table, color);
        match result {
            Ok(_) => self
                .color_tables
                .history
                .record(before, &self.workspaces.store.current.color_tables),
            Err(e) => self.status = Some(e),
        }
    }
    pub(super) fn color_table_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Local swatches and mixing palette", |ui| {
            let before = self.workspaces.store.current.color_tables.clone();
            let mut chosen = None;
            let mut error = None;
            let mut restore = 0;
            let current = self.references.session.workspace.color;
            let recent = self.workspaces.store.current.eyedropper.recent.clone();
            let tables = &mut self.workspaces.store.current.color_tables;
            egui::ComboBox::from_id_salt("color-table")
                .selected_text(
                    tables
                        .tables
                        .iter()
                        .find(|t| Some(t.id) == tables.selected)
                        .map_or("No local table", |t| t.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    for t in &tables.tables {
                        ui.selectable_value(&mut tables.selected, Some(t.id), &t.name);
                    }
                });
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.color_tables.name);
                if ui.button("New table").clicked()
                    && let Err(e) = tables.create(&self.color_tables.name)
                {
                    error = Some(e)
                }
                if ui.button("Delete selected table").clicked()
                    && let Some(id) = tables.selected
                    && let Err(e) = tables.remove(id)
                {
                    error = Some(e)
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Add current RGBA").clicked()
                    && let Err(e) = tables.add(current)
                {
                    error = Some(e)
                }
                if ui.button("Copy used colors into table").clicked() {
                    let mut next = tables.clone();
                    let result = recent.iter().try_for_each(|c| next.add(*c).map(|_| ()));
                    match result {
                        Ok(()) => *tables = next,
                        Err(e) => error = Some(e),
                    }
                }
            });
            ui.checkbox(
                &mut tables.auto_sample,
                "Register sampler color on pointer release (Escape cancels)",
            );
            super::edit::number(
                ui,
                &mut tables.history_size,
                12.0..=64.0,
                1.0,
                " used-color tile size",
                "color-tables",
            );
            super::edit::number(
                ui,
                &mut tables.history_columns,
                1.0..=32.0,
                1.0,
                " used-color columns",
                "color-tables",
            );
            if let Some(t) = tables
                .tables
                .iter_mut()
                .find(|t| Some(t.id) == tables.selected)
            {
                ui.text_edit_singleline(&mut t.name);
                let mut remove = None;
                let mut movement = None;
                for c in &t.colors {
                    ui.push_id(c.id, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new(&c.name).fill(
                                    egui::Color32::from_rgba_unmultiplied(
                                        c.color[0], c.color[1], c.color[2], c.color[3],
                                    ),
                                ))
                                .clicked()
                            {
                                chosen = Some(c.color)
                            }
                            ui.selectable_value(
                                &mut self.color_tables.selected,
                                Some(c.id),
                                "Select",
                            );
                            if ui.small_button("Earlier").clicked() {
                                movement = Some((c.id, -1))
                            }
                            if ui.small_button("Later").clicked() {
                                movement = Some((c.id, 1))
                            }
                            if ui.small_button("Remove").clicked() {
                                remove = Some(c.id)
                            }
                        });
                    });
                }
                if let Some(id) = remove
                    && let Err(e) = tables.delete_color(id)
                {
                    error = Some(e)
                }
                if let Some((id, delta)) = movement
                    && let Err(e) = tables.move_color(id, delta)
                {
                    error = Some(e)
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Undo color table edit").clicked() {
                    restore = -1
                }
                if ui.button("Redo color table edit").clicked() {
                    restore = 1
                }
            });
            if !tables.valid() {
                *tables = before.clone();
                error = Some("Invalid color table name or layout; previous values kept".into());
            }
            self.color_tables.history.record(before, tables);
            if restore < 0 {
                self.color_tables.history.undo(tables);
            }
            if restore > 0 {
                self.color_tables.history.redo(tables);
            }
            ui.horizontal(|ui| {
                if ui.button("Clear used-color list").clicked()
                    && !self.workspaces.store.current.eyedropper.recent.is_empty()
                {
                    self.color_tables.recent_restore = Some(std::mem::take(
                        &mut self.workspaces.store.current.eyedropper.recent,
                    ));
                }
                if ui
                    .add_enabled(
                        self.color_tables.recent_restore.is_some(),
                        egui::Button::new("Restore cleared used colors"),
                    )
                    .clicked()
                {
                    self.workspaces.store.current.eyedropper.recent =
                        self.color_tables.recent_restore.take().unwrap();
                }
            });
            if ui.button("Open local mixing palette").clicked() {
                self.workspaces.store.current.mixer.visible = true;
            }
            if ui.button("Open independent test painting").clicked() {
                self.workspaces.store.current.scratchpad.visible = true;
            }
            if let Some(c) = chosen {
                self.choose_color(c)
            }
            if let Some(e) = error {
                self.status = Some(e)
            }
        });
    }
}
