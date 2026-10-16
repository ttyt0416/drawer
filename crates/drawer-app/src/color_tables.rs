use super::*;
use drawer_core::color_tables::History;
#[derive(Default)]
pub(super) struct Tables {
    pub history: History,
    recent_restore: Option<Vec<[u8; 4]>>,
    selected: Option<u64>,
    name: String,
    edit_name: String,
    editing: Option<u64>,
    drag: Option<(u64, bool)>,
    rows: Vec<(u64, egui::Rect, egui::LayerId)>,
    table: Option<u64>,
}
impl Tables {
    pub(super) fn cancel_drag(&mut self) {
        self.drag = None;
    }
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
        let shown = ui.collapsing("Local swatches and mixing palette", |ui| {
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
            match table_ui(ui, tables, &mut self.color_tables, current) {
                Ok(Some(c)) => chosen = Some(c),
                Ok(None) => {}
                Err(e) => error = Some(e),
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
            if restore != 0 {
                self.color_tables.drag = None;
            }
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
        if shown.body_returned.is_none() || !ui.input(|i| i.pointer.primary_down()) {
            self.color_tables.drag = None;
        }
    }
}

fn table_ui(
    ui: &mut egui::Ui,
    tables: &mut drawer_core::color_tables::Settings,
    state: &mut Tables,
    current: [u8; 4],
) -> Result<Option<[u8; 4]>, String> {
    if state.table != tables.selected {
        state.drag = None;
        state.table = tables.selected;
    }
    let Some(index) = tables
        .tables
        .iter()
        .position(|t| Some(t.id) == tables.selected)
    else {
        state.drag = None;
        state.rows.clear();
        return Ok(None);
    };
    let t = &mut tables.tables[index];
    ui.text_edit_singleline(&mut t.name);
    let layout = &mut t.layout;
    ui.horizontal_wrapped(|ui| {
        super::edit::number(
            ui,
            &mut layout.size,
            12.0..=64.0,
            1.0,
            " swatch px",
            "color-tables",
        );
        let mut auto = layout.columns.is_none();
        if ui.checkbox(&mut auto, "Automatic columns").changed() {
            layout.columns = if auto { None } else { Some(8) }
        }
        if let Some(n) = &mut layout.columns {
            super::edit::number(ui, n, 1.0..=32.0, 1.0, " fixed columns", "color-tables");
        }
        ui.checkbox(&mut layout.scroll, "Scroll swatches");
    });
    let layout = *layout;
    let columns = layout.columns_for(ui.available_width());
    let mut chosen = None;
    state.rows.clear();
    let mut draw = |ui: &mut egui::Ui| {
        egui::Grid::new("editable-swatches")
            .spacing(egui::vec2(2.0, 2.0))
            .show(ui, |ui| {
                for (n, c) in t.colors.iter().enumerate() {
                    ui.push_id(c.id, |ui| {
                        let response = ui
                            .add_sized(
                                [layout.size as f32; 2],
                                egui::Button::new(" ")
                                    .fill(egui::Color32::from_rgba_unmultiplied(
                                        c.color[0], c.color[1], c.color[2], c.color[3],
                                    ))
                                    .sense(egui::Sense::click_and_drag()),
                            )
                            .on_hover_text(format!(
                                "{} · {} · drag to move; Ctrl-drag to copy",
                                c.name,
                                drawer_core::color_selector::hex(c.color)
                            ));
                        state.rows.push((
                            c.id,
                            response.rect.intersect(ui.clip_rect()),
                            ui.layer_id(),
                        ));
                        if response.clicked() {
                            state.selected = Some(c.id);
                            chosen = Some(c.color);
                        }
                        if response.drag_started() {
                            state.drag = Some((c.id, ui.input(|i| i.modifiers.ctrl)));
                        }
                        if response.dragged() {
                            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                        }
                    });
                    if (n + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
    };
    if layout.scroll {
        egui::ScrollArea::both()
            .id_salt("swatches-scroll")
            .max_height(240.0)
            .show(ui, |ui| draw(ui));
    } else {
        draw(ui);
    }
    if ui.input(|i| {
        !i.focused
            || i.key_pressed(egui::Key::Escape)
            || i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerGone | egui::Event::WindowFocused(false)
                )
            })
    }) {
        state.drag = None;
    }
    if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
        && let Some((source, copy)) = state.drag.take()
    {
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        if !copy || ctrl {
            let target = ui.input(|i| i.pointer.latest_pos()).and_then(|p| {
                state
                    .rows
                    .iter()
                    .find(|(_, r, l)| r.contains(p) && ui.ctx().layer_id_at(p) == Some(*l))
                    .map(|(id, _, _)| *id)
            });
            if let Some(target) = target {
                state.selected = Some(tables.drop_before(source, target, copy)?);
            }
        }
    }
    if let Some(c) = state
        .selected
        .and_then(|id| tables.tables[index].colors.iter().find(|c| c.id == id))
        .cloned()
    {
        if state.editing != Some(c.id) {
            state.editing = Some(c.id);
            state.edit_name = c.name.clone();
        }
        super::edit::text(ui, &mut state.edit_name, "color-tables");
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            for (n, label) in [
                "Rename selected swatch",
                "Replace selected swatch with current RGBA",
                "Earlier",
                "Later",
                "Remove selected swatch",
            ]
            .iter()
            .enumerate()
            {
                if ui.button(*label).clicked() {
                    action = Some(n);
                }
            }
        });
        match action {
            Some(0) => tables.edit_color(c.id, &state.edit_name, c.color)?,
            Some(1) => tables.edit_color(c.id, &c.name, current)?,
            Some(2) => tables.move_color(c.id, -1)?,
            Some(3) => tables.move_color(c.id, 1)?,
            Some(4) => {
                tables.delete_color(c.id)?;
                state.selected = None;
            }
            _ => {}
        }
    }
    Ok(chosen)
}

fn editable_frame(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: f64,
    width: f32,
    events: Vec<egui::Event>,
) -> Result<(), String> {
    let mut result = Ok(());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 800.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| {
            let tables = &mut app.workspaces.store.current.color_tables;
            let before = tables.clone();
            match table_ui(ui, tables, &mut app.color_tables, [22, 44, 66, 128]) {
                Ok(Some(c)) => app.references.session.workspace.color = c,
                Ok(None) => {}
                Err(e) => result = Err(e),
            }
            app.color_tables.history.record(before, tables);
        },
    );
    output.textures_delta.clear();
    result
}
fn editable_pointer(p: egui::Pos2, button: Option<bool>, ctrl: bool) -> Vec<egui::Event> {
    let modifiers = egui::Modifiers {
        ctrl,
        command: ctrl,
        ..Default::default()
    };
    let mut events = vec![
        egui::Event::ModifiersChanged(modifiers),
        egui::Event::PointerMoved(p),
    ];
    if let Some(pressed) = button {
        events.push(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    events
}
pub(super) fn smoke_editable(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let user = root.join("editable-colors-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 2,
        height: 2,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    drawer_core::files::save(
        &mut app.documents[0],
        &root.join("swatches-keeps-document.drawer"),
    )
    .map_err(|e| e.to_string())?;
    let document = app.documents[0].snapshot();
    let table = &mut app.workspaces.store.current.color_tables;
    let a = table.add([1, 2, 3, 0])?;
    let b = table.add([4, 5, 6, 128])?;
    let c = table.add([7, 8, 9, 255])?;
    editable_frame(&ctx, &mut app, 0.0, 700.0, vec![])?;
    editable_frame(&ctx, &mut app, 0.1, 700.0, vec![])?;
    let row = |app: &Drawer, id| {
        app.color_tables
            .rows
            .iter()
            .find(|(n, _, _)| *n == id)
            .map(|(_, r, _)| r.center())
            .ok_or("Swatch row missing".to_string())
    };
    let start = row(&app, c)?;
    let target = row(&app, a)?;
    editable_frame(
        &ctx,
        &mut app,
        0.2,
        700.0,
        editable_pointer(start, Some(true), false),
    )?;
    editable_frame(
        &ctx,
        &mut app,
        0.3,
        700.0,
        editable_pointer(target, None, false),
    )?;
    editable_frame(
        &ctx,
        &mut app,
        0.4,
        700.0,
        editable_pointer(target, Some(false), false),
    )?;
    if app.workspaces.store.current.color_tables.tables[0]
        .colors
        .iter()
        .map(|c| c.id)
        .collect::<Vec<_>>()
        != vec![c, a, b]
    {
        return Err("Actual swatch drag move failed".into());
    }
    editable_frame(&ctx, &mut app, 0.45, 700.0, vec![])?;
    let start = row(&app, a)?;
    let target = row(&app, b)?;
    editable_frame(
        &ctx,
        &mut app,
        0.5,
        700.0,
        editable_pointer(start, Some(true), true),
    )?;
    editable_frame(
        &ctx,
        &mut app,
        0.6,
        700.0,
        editable_pointer(target, None, true),
    )?;
    editable_frame(
        &ctx,
        &mut app,
        0.7,
        700.0,
        editable_pointer(target, Some(false), true),
    )?;
    let after = app.workspaces.store.current.color_tables.clone();
    if after.tables[0].colors.len() != 4
        || after.tables[0]
            .colors
            .iter()
            .filter(|c| c.color == [1, 2, 3, 0])
            .count()
            != 2
    {
        return Err("Actual Ctrl swatch copy failed".into());
    }
    if !app
        .color_tables
        .history
        .undo(&mut app.workspaces.store.current.color_tables)
        || app.workspaces.store.current.color_tables.tables[0]
            .colors
            .len()
            != 3
        || !app
            .color_tables
            .history
            .redo(&mut app.workspaces.store.current.color_tables)
        || app.workspaces.store.current.color_tables != after
    {
        return Err("Swatch drag undo/redo failed".into());
    }
    editable_frame(&ctx, &mut app, 0.8, 700.0, vec![])?;
    for n in 0..3 {
        let start = row(&app, a)?;
        let outside = if n == 0 {
            egui::pos2(690.0, 790.0)
        } else {
            row(&app, b)?
        };
        let t = 1.0 + n as f64;
        editable_frame(
            &ctx,
            &mut app,
            t,
            700.0,
            editable_pointer(start, Some(true), true),
        )?;
        editable_frame(
            &ctx,
            &mut app,
            t + 0.1,
            700.0,
            editable_pointer(outside, None, true),
        )?;
        let mut events = editable_pointer(outside, Some(false), n != 2);
        if n == 1 {
            events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        editable_frame(&ctx, &mut app, t + 0.2, 700.0, events)?;
        if app.workspaces.store.current.color_tables != after || app.color_tables.drag.is_some() {
            return Err("Outside/Escape swatch drop changed table".into());
        }
    }
    app.workspaces.store.current.color_tables.edit_color(
        a,
        "Named local color",
        [22, 44, 66, 128],
    )?;
    app.workspaces.store.current.color_tables.tables[0].layout =
        drawer_core::color_tables::Layout {
            size: 40,
            columns: Some(2),
            scroll: true,
        };
    editable_frame(&ctx, &mut app, 4.3, 180.0, vec![])?;
    let expected = app.workspaces.store.current.color_tables.clone();
    app.persist_workspace()?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().color_tables != expected {
        return Err("Swatch values/order/layout restart failed".into());
    }
    if app.documents[0].snapshot() != document
        || app.documents[0].dirty
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Swatch edits changed document".into());
    }
    Ok(())
}
#[cfg(test)]
mod editable_tests {
    #[test]
    fn actual_drag_move_ctrl_copy_cancel_undo_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke_editable(&root.0).unwrap();
    }
}
