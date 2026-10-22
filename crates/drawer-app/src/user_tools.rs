use super::*;
use drawer_core::{
    brush_presets::Preset, tool_properties::Property, user_tools::Library, workspaces::Snapshot,
};
use std::collections::BTreeSet;
pub(super) enum Action {
    Brush(Box<Preset>),
    Backup(Box<Snapshot>),
}
pub(super) struct Editor {
    pub draft: Library,
    pub history: drawer_core::user_tools::History,
    drag: Option<(u64, bool)>,
    #[cfg(test)]
    rows: Vec<(u64, egui::Rect, egui::Rect)>,
    pub original_active: Option<u64>,
    pub backup: Option<Box<Snapshot>>,
    selected: BTreeSet<u64>,
    category: Option<u64>,
    destination: Option<u64>,
    name: String,
    category_name: String,
    base: BrushTip,
    properties: Vec<Property>,
    replace: bool,
    message: Option<String>,
}
impl Editor {
    pub fn new(draft: Library, base: BrushTip) -> Self {
        Self {
            original_active: draft.active,
            category: draft.categories.first().map(|c| c.id),
            destination: None,
            draft,
            history: Default::default(),
            drag: None,
            #[cfg(test)]
            rows: vec![],
            backup: None,
            selected: Default::default(),
            name: "New tool".into(),
            category_name: "New category".into(),
            base,
            properties: Property::ALL.to_vec(),
            replace: false,
            message: None,
        }
    }
    fn finish_drag(
        &mut self,
        target: Option<u64>,
        ctrl: bool,
        focused: bool,
        cancel: bool,
    ) -> Result<Option<u64>, String> {
        let Some((source, copy)) = self.drag.take() else {
            return Ok(None);
        };
        if cancel || !focused || (copy && !ctrl) {
            return Ok(None);
        }
        let Some(target) = target else {
            return Ok(None);
        };
        self.draft.drop_before(source, target, copy).map(Some)
    }
    fn show_result<T>(&mut self, result: Result<T, String>) {
        self.message = Some(match result {
            Ok(_) => {
                "Tool draft updated. Apply saves it; Cancel preserves the current settings.".into()
            }
            Err(e) => e,
        });
    }
}
impl Drawer {
    pub(super) fn select_user_tool(&mut self, id: u64) -> Result<(), String> {
        if self.defer_path(drawing::path_draft::Action::UserTool(id)) {
            return Ok(());
        }

        if self.workspaces.store.current.user_tools.active == Some(id) {
            self.drawing.mode = super::precision::Mode::Paint;
            return Ok(());
        }
        self.restore_locked_tool();
        self.finish_stroke();
        self.sync_tool_properties();
        let current = self.capture_workspace();
        let next = current.user_tools.activate(&current, id)?;
        let mode = self.drawing.mode;
        self.restore_workspace(&next);
        self.workspaces.store.current = next;
        self.drawing.mode = super::precision::Mode::Paint;
        self.precision.finish();
        self.sync_tool_properties();
        if let Err(e) = self.persist_workspace() {
            self.restore_workspace(&current);
            self.workspaces.store.current = current;
            self.drawing.mode = mode;
            return Err(e);
        }
        Ok(())
    }
    pub(super) fn leave_user_tool(&mut self) {
        if self.workspaces.store.current.user_tools.active.is_none() {
            return;
        }
        self.finish_stroke();
        self.sync_tool_properties();
        let current = self.capture_workspace();
        match current.user_tools.deactivate(&current) {
            Ok(next) => {
                self.restore_workspace(&next);
                self.workspaces.store.current = next;
            }
            Err(e) => self.status = Some(e),
        }
    }
    pub(super) fn user_tool_supports(&self, property: Property) -> bool {
        self.workspaces
            .store
            .current
            .user_tools
            .active_entry()
            .is_none_or(|t| t.preset.tip != self.selected || t.properties.contains(&property))
    }
    pub(super) fn user_tool_picker(&mut self, ui: &mut egui::Ui) {
        let library = self.capture_workspace().user_tools;
        let mut choice = None;
        egui::ComboBox::from_id_salt("user-tool-picker")
            .selected_text(library.active_entry().map_or_else(
                || "User tools".into(),
                |t| {
                    library
                        .command_label(&drawer_core::user_tools::command(t.id))
                        .unwrap()
                },
            ))
            .show_ui(ui, |ui| {
                for cat in &library.categories {
                    ui.label(&cat.name);
                    for tool in library.tools.iter().filter(|t| t.category == cat.id) {
                        if ui
                            .selectable_label(library.active == Some(tool.id), &tool.preset.name)
                            .clicked()
                        {
                            choice = Some(tool.id);
                        }
                    }
                }
            });
        if let Some(id) = choice
            && let Err(e) = self.select_user_tool(id)
        {
            self.status = Some(e);
        }
        if ui.button("Manage local tools…").clicked() {
            self.finish_stroke();
            self.brush_presets.open = true;
            self.brush_presets.tools_requested = true;
        }
    }
    fn tool_slot_rows(&mut self, ui: &mut egui::Ui, e: &mut Editor) {
        ui.label("Drag a handle to insert before another slot. Hold Ctrl throughout a drag to copy. Escape, focus loss or releasing Ctrl cancels; drop outside the list cancels.");
        #[cfg(test)]
        {
            e.rows.clear();
        }
        let mut target = None;
        let position = ui.input(|i| i.pointer.interact_pos());
        for t in e
            .draft
            .tools
            .iter()
            .filter(|t| Some(t.category) == e.category)
        {
            let response = ui.horizontal(|ui| {
                let handle = ui.add(egui::Label::new("::").sense(egui::Sense::drag()));
                if handle.drag_started_by(egui::PointerButton::Primary) {
                    e.drag = Some((t.id, ui.input(|i| i.modifiers.ctrl)));
                }
                let mut selected = e.selected.contains(&t.id);
                if ui
                    .checkbox(
                        &mut selected,
                        format!("{} · {}", t.preset.name, t.preset.tip.label()),
                    )
                    .changed()
                {
                    if selected {
                        e.selected.insert(t.id);
                        e.name = t.preset.name.clone();
                        e.properties = t.properties.clone();
                    } else {
                        e.selected.remove(&t.id);
                    }
                }
                handle.rect
            });
            #[cfg(test)]
            e.rows.push((t.id, response.response.rect, response.inner));
            if position.is_some_and(|p| response.response.rect.contains(p)) {
                target = Some(t.id);
                if e.drag.is_some() {
                    ui.painter().hline(
                        response.response.rect.x_range(),
                        response.response.rect.top(),
                        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                    );
                }
            }
        }
        let (released, ctrl, focused, escape) = ui.input(|i| {
            (
                i.pointer.button_released(egui::PointerButton::Primary),
                i.modifiers.ctrl,
                i.focused,
                i.key_pressed(egui::Key::Escape),
            )
        });
        if e.drag.is_some() && (escape || !focused || e.drag.is_some_and(|(_, copy)| copy && !ctrl))
        {
            let _ = e.finish_drag(None, ctrl, focused, true);
            if escape {
                ui.input_mut(|i| {
                    let modifiers = i.modifiers;
                    i.consume_key(modifiers, egui::Key::Escape);
                });
            }
            e.message = Some("Slot drag cancelled; list preserved.".into());
        } else if released {
            let r = e.finish_drag(target, ctrl, focused, false);
            if let Ok(Some(id)) = &r {
                e.selected = BTreeSet::from([*id]);
            }
            if r.as_ref().is_err() || r.as_ref().is_ok_and(|v| v.is_some()) {
                e.show_result(r);
            }
        }
    }
    pub(super) fn user_tools_ui(&mut self, ui: &mut egui::Ui, e: &mut Editor) -> Option<Action> {
        let mut action = None;
        let mut history_action = false;
        ui.horizontal(|ui| {
            let (undo, redo) = e.history.counts();
            if ui
                .add_enabled(undo > 0, egui::Button::new("Undo tool list"))
                .clicked()
            {
                e.history.undo(&mut e.draft);
                history_action = true;
            }
            if ui
                .add_enabled(redo > 0, egui::Button::new("Redo tool list"))
                .clicked()
            {
                e.history.redo(&mut e.draft);
                history_action = true;
            }
        });
        let before = e.draft.clone();
        let current = self.capture_workspace();
        ui.label("Independent local tools use one of the seven brush engines. Their names, IDs, categories, defaults and supported common properties are saved with settings. Picture undo stays separate.");
        if let Some(backup) = &e.backup {
            ui.colored_label(egui::Color32::YELLOW,format!("A full settings backup, including {} tools and their command references, is staged for Apply.",backup.user_tools.tools.len()));
        }
        egui::ComboBox::from_id_salt("user-category")
            .selected_text(
                e.category
                    .and_then(|id| e.draft.categories.iter().find(|c| c.id == id))
                    .map_or("No category", |c| c.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for c in &e.draft.categories {
                    if ui
                        .selectable_value(&mut e.category, Some(c.id), &c.name)
                        .changed()
                    {
                        e.category_name = c.name.clone();
                    }
                }
            });
        ui.label("Category name");
        super::edit::text(ui, &mut e.category_name, "user-tools");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Create category").clicked() {
                let result = e.draft.create_category(&e.category_name);
                if let Ok(id) = result.as_ref() {
                    e.category = Some(*id);
                }
                e.show_result(result);
            }
            if ui
                .add_enabled(e.category.is_some(), egui::Button::new("Rename category"))
                .clicked()
            {
                let result = e
                    .draft
                    .rename_category(e.category.unwrap(), &e.category_name);
                e.show_result(result);
            }
            for (delta, label) in [(-1, "Category earlier"), (1, "Category later")] {
                if ui
                    .add_enabled(e.category.is_some(), egui::Button::new(label))
                    .clicked()
                {
                    let result = e.draft.reorder_category(e.category.unwrap(), delta);
                    e.show_result(result);
                }
            }
        });
        ui.separator();
        self.tool_slot_rows(ui, e);
        ui.horizontal(|ui| {
            ui.label(format!("{} tools selected", e.selected.len()));
            if ui.button("Select this category").clicked() {
                e.selected.extend(
                    e.draft
                        .tools
                        .iter()
                        .filter(|t| Some(t.category) == e.category)
                        .map(|t| t.id),
                );
            }
            if ui.button("Clear tool selection").clicked() {
                e.selected.clear();
            }
        });
        ui.label("Tool name");
        super::edit::text(ui, &mut e.name, "user-tools");
        egui::ComboBox::from_id_salt("user-base-engine")
            .selected_text(e.base.label())
            .show_ui(ui, |ui| {
                for tip in BrushTip::ALL {
                    ui.selectable_value(&mut e.base, tip, tip.label());
                }
            });
        ui.label("Supported common properties (unsupported values use the factory value)");
        ui.horizontal_wrapped(|ui| {
            for p in Property::ALL {
                let mut enabled = e.properties.contains(&p);
                if ui.checkbox(&mut enabled, p.name()).changed() {
                    if enabled {
                        e.properties.push(p);
                    } else {
                        e.properties.retain(|v| *v != p);
                    }
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (factory, label) in [
                (false, "Create tool from current base settings"),
                (true, "Create tool from factory base"),
            ] {
                if ui
                    .add_enabled(e.category.is_some(), egui::Button::new(label))
                    .clicked()
                {
                    let result = (|| {
                        let preset = if factory {
                            let mut p = Library::default().base_default(&current, e.base)?;
                            p.name = e.name.trim().into();
                            p
                        } else {
                            Preset::capture(&current, e.base, &e.name)?
                        };
                        e.draft
                            .create_tool(e.category.unwrap(), preset, e.properties.clone())
                    })();
                    if let Ok(id) = result.as_ref() {
                        e.selected = BTreeSet::from([*id]);
                    }
                    e.show_result(result);
                }
            }
        });
        let ids = e
            .draft
            .tools
            .iter()
            .filter(|t| e.selected.contains(&t.id))
            .map(|t| t.id)
            .collect::<Vec<_>>();
        let single = if ids.len() == 1 {
            ids.first().copied()
        } else {
            None
        };
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(single.is_some(), egui::Button::new("Rename selected tool"))
                .clicked()
            {
                let r = e.draft.rename_tool(single.unwrap(), &e.name);
                e.show_result(r);
            }
            if ui
                .add_enabled(
                    single.is_some(),
                    egui::Button::new("Set supported properties"),
                )
                .clicked()
            {
                let r = e.draft.capabilities(single.unwrap(), e.properties.clone());
                e.show_result(r);
            }
            if ui
                .add_enabled(
                    single.is_some(),
                    egui::Button::new("Register selected values as default"),
                )
                .clicked()
            {
                let r = e.draft.register_default(single.unwrap());
                e.show_result(r);
            }
            if ui
                .add_enabled(
                    single.is_some(),
                    egui::Button::new("Reset selected tool to its default"),
                )
                .clicked()
            {
                let r = e.draft.reset_tool(single.unwrap());
                e.show_result(r);
            }
            if let Some(id) = single {
                let mut locked = e.draft.entry(id).unwrap().preset.locked;
                if ui
                    .checkbox(&mut locked, "Lock selected stored preset")
                    .changed()
                {
                    let r = e.draft.lock_tool(id, locked, None);
                    e.show_result(r);
                }
            }
            for (delta, label) in [(-1, "Tool earlier"), (1, "Tool later")] {
                if ui
                    .add_enabled(single.is_some(), egui::Button::new(label))
                    .clicked()
                {
                    let r = e.draft.reorder_tool(single.unwrap(), delta);
                    e.show_result(r);
                }
            }
        });
        egui::ComboBox::from_id_salt("user-tool-destination")
            .selected_text(
                e.destination
                    .and_then(|id| e.draft.categories.iter().find(|c| c.id == id))
                    .map_or("Choose destination", |c| c.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for c in &e.draft.categories {
                    ui.selectable_value(&mut e.destination, Some(c.id), &c.name);
                }
            });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !ids.is_empty() && e.destination.is_some(),
                    egui::Button::new("Move selected tools"),
                )
                .clicked()
            {
                let r = e.draft.move_tools(&ids, e.destination.unwrap());
                e.show_result(r);
            }
            if ui
                .add_enabled(
                    !ids.is_empty() && e.destination.is_some(),
                    egui::Button::new("Duplicate selected tools"),
                )
                .clicked()
            {
                let r = e.draft.duplicate(&ids, e.destination.unwrap());
                if let Ok(created) = r.as_ref() {
                    e.selected = created.iter().copied().collect();
                }
                e.show_result(r);
            }
            if ui
                .add_enabled(!ids.is_empty(), egui::Button::new("Delete selected tools"))
                .clicked()
            {
                let r = e.draft.delete_tools(&ids);
                if r.is_ok() {
                    e.selected.clear();
                }
                e.show_result(r);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (move_tools, label) in [
                (true, "Delete category and move tools"),
                (false, "Delete category and its tools"),
            ] {
                if ui
                    .add_enabled(
                        e.category.is_some() && (!move_tools || e.destination.is_some()),
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    let r = e.draft.delete_category(
                        e.category.unwrap(),
                        if move_tools { e.destination } else { None },
                    );
                    if r.is_ok() {
                        e.category = e.draft.categories.first().map(|c| c.id);
                        e.selected.retain(|id| e.draft.entry(*id).is_some());
                    }
                    e.show_result(r);
                }
            }
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Register current base settings as user default")
                .clicked()
            {
                let r = Preset::capture(&current, e.base, &format!("Default {}", e.base.label()))
                    .and_then(|p| e.draft.register_base_default(p));
                e.show_result(r);
            }
            if ui.button("Stage restore of base default").clicked() {
                match e.draft.base_default(&current, e.base) {
                    Ok(p) => action = Some(Action::Brush(Box::new(p))),
                    Err(err) => e.message = Some(err),
                }
            }
        });
        ui.label("Resetting tools preserves saved brush presets and source materials. Deleting an active user tool switches to the factory Pen. Missing command references stay visible for repair.");
        crate::help::checkbox(
            ui,
            &mut e.replace,
            "Replace existing exported group file",
            "user-tools",
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Export selected tools / current category…")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("tool-group.json")
                    .add_filter("Drawer local tool group", &["json"])
                    .save_file()
            {
                let r = e
                    .draft
                    .group(&ids, if ids.is_empty() { e.category } else { None })
                    .and_then(|group| group.save(&path, e.replace).map_err(|err| err.to_string()));
                e.show_result(r);
            }
            if ui.button("Import local tool group into draft…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Drawer local tool group", &["json"])
                    .pick_file()
            {
                let r = Library::read(&path)
                    .map_err(|err| err.to_string())
                    .and_then(|group| e.draft.import_group(&group));
                e.show_result(r);
            }
            if ui
                .add_enabled(
                    self.storage.is_some(),
                    egui::Button::new("Stage tool + reference settings backup"),
                )
                .clicked()
            {
                match Snapshot::read(
                    &self
                        .storage
                        .as_ref()
                        .unwrap()
                        .root()
                        .join("settings/workspace-before-apply.json"),
                ) {
                    Ok(snapshot) => {
                        e.draft = snapshot.user_tools.clone();
                        e.category = e.draft.categories.first().map(|c| c.id);
                        e.selected.clear();
                        action = Some(Action::Backup(Box::new(snapshot)));
                    }
                    Err(err) => e.message = Some(err.to_string()),
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Export portable tools + materials…").clicked()
                && let Some(parent) = rfd::FileDialog::new().pick_folder()
            {
                let r = e
                    .draft
                    .group(&ids, if ids.is_empty() { e.category } else { None })
                    .and_then(|group| {
                        drawer_core::tool_bundle::export(
                            &parent.join("Drawer-tool-bundle"),
                            &group,
                            &self.materials.library,
                        )
                        .map_err(|e| e.to_string())
                    });
                e.show_result(r);
            }
            if ui.button("Import portable tool bundle…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                let r = drawer_core::tool_bundle::import(&path, &e.draft, &self.materials.library)
                    .map_err(|e| e.to_string())
                    .and_then(|(tools, materials)| {
                        self.publish_materials(materials)?;
                        e.draft = tools;
                        Ok(())
                    });
                e.show_result(r);
            }
        });
        ui.label("Portable export creates a new Drawer-tool-bundle folder inside the chosen folder. Import validates all files before registering materials and staging slots. Apply saves slots; Cancel keeps the previous slots. Registered materials remain local for reuse.");
        ui.label("Group JSON remaps imported tool/category IDs and preserves empty categories. Material IDs reference the local image library; image files are not included in this group format.");
        let mut preview = current.clone();
        preview.user_tools = e.draft.clone();
        for report in e.draft.missing_refs(&preview) {
            ui.colored_label(egui::Color32::YELLOW, report);
        }
        for t in e.draft.tools.iter().filter(|t| e.selected.contains(&t.id)) {
            for id in t.preset.sequence.missing().into_iter().filter(|id| {
                self.materials
                    .library
                    .get(*id, drawer_core::material_library::Kind::Tip)
                    .is_none()
            }) {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("{}: missing tip {id}", t.preset.name),
                );
            }
        }
        if let Some(message) = &e.message {
            ui.label(message);
        }
        e.selected.retain(|id| e.draft.entry(*id).is_some());
        if !history_action {
            e.history.record(before, &e.draft);
        }
        action
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use super::input_replay::frame;
    use drawer_core::{
        actions::Item, material_library::Kind, pointer_input::Phase, user_tools::command,
    };
    let ctx = egui::Context::default();
    let user = root.join("user-tools-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 16,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    frame(&ctx, &mut app, 0.0, vec![]);
    frame(&ctx, &mut app, 0.1, vec![]);
    let original = app.documents[0].snapshot();
    let baseline = app.capture_workspace();
    let library = app
        .materials
        .library
        .register_mask("User tool source", Kind::Tip, 2, 2, vec![255, 0, 0, 255])
        .map_err(|e| e.to_string())?;
    app.publish_materials(library)?;
    let materials = app.materials.library.clone();
    let asset = materials.assets()[0].id;
    let mut next = baseline.clone();
    let mut a = Preset::capture(&next, BrushTip::Pen, "Ink A")?;
    a.values[0] = 5.0;
    a.values[4] = 0.4;
    a.sequence = drawer_core::tip_sequence::Settings {
        enabled: true,
        count: 1,
        tips: [asset, 0, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    let mut b = a.clone();
    b.name = "Ink B".into();
    b.values[0] = 9.0;
    b.values[4] = 0.8;
    let aid = next.user_tools.create_tool(1, a, Property::ALL.to_vec())?;
    let bid = next.user_tools.create_tool(1, b, Property::ALL.to_vec())?;
    let mut fixed = Preset::capture(&next, BrushTip::Pen, "Fixed")?;
    fixed.values[0] = 99.0;
    let fixed_id = next
        .user_tools
        .create_tool(1, fixed, vec![Property::Opacity])?;
    next.brushes.put(
        Preset::capture(&next, BrushTip::Pencil, "Keep saved preset")?,
        false,
    )?;
    let acmd = command(aid);
    next.bindings.push(drawer_core::workspaces::Binding {
        key: "F4".into(),
        ctrl: false,
        shift: false,
        alt: false,
        command: acmd.clone(),
    });
    next.actions.access[0]
        .items
        .push(Item::Command(acmd.clone()));
    app.stage_workspace(next, app.workspaces.store.active.clone());
    app.apply_workspace(drawer_core::workspaces::Scope::ALL, None)?;
    app.select_user_tool(aid)?;
    if app.panels.settings.brush_size != 5.0 {
        return Err("User tool A did not activate".into());
    }
    app.set_quick_brush_size(7.0)?;
    app.sync_tool_properties();
    app.select_user_tool(bid)?;
    if app.panels.settings.brush_size != 9.0
        || app
            .capture_workspace()
            .user_tools
            .entry(aid)
            .unwrap()
            .preset
            .values[0]
            != 7.0
    {
        return Err("User tools sharing a base did not keep independent edits".into());
    }
    app.set_tool(BrushTip::Pen);
    if app.panels.settings.brush_size != baseline.panels.brush_size
        || app.workspaces.store.current.user_tools.active.is_some()
    {
        return Err("Builtin brush was not restored after user tool".into());
    }
    app.select_user_tool(fixed_id)?;
    if app.panels.settings.brush_size != 12.0
        || app.set_quick_brush_size(33.0).is_ok()
        || app.command_reason("brush-grow").is_none()
    {
        return Err("Unsupported user tool property remained editable".into());
    }
    let key = |pressed| egui::Event::Key {
        key: egui::Key::F4,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    frame(&ctx, &mut app, 0.18, vec![key(true)]);
    frame(&ctx, &mut app, 0.19, vec![key(false)]);
    if app.workspaces.store.current.user_tools.active != Some(aid)
        || app.panels.settings.brush_size != 7.0
        || app.command_reason(&acmd).is_some()
    {
        return Err("Stable user tool command did not select saved instance".into());
    }
    app.workspaces.store.current.actions.cycles[0].tools = vec!["Pen".into(), acmd.clone()];
    app.execute_command(&ctx, "cycle-next:1");
    if app.workspaces.store.current.user_tools.active.is_some() {
        return Err("Tool cycle did not return to builtin brush".into());
    }
    app.execute_command(&ctx, "cycle-prev:1");
    if app.workspaces.store.current.user_tools.active != Some(aid) {
        return Err("Tool cycle did not select user tool instance".into());
    }
    app.workspaces
        .store
        .current
        .user_tools
        .rename_tool(aid, "Ink A renamed")?;
    if app.command_reason(&acmd).is_some()
        || !app
            .capture_workspace()
            .command_label(&acmd)
            .contains("renamed")
    {
        return Err("Renamed tool broke stable command reference".into());
    }
    frame(&ctx, &mut app, 0.2, vec![]);
    let events = vec![
        super::precision::pointer(&ctx, &app, [5.5, 8.5], Phase::Down),
        super::precision::pointer(&ctx, &app, [25.5, 8.5], Phase::Up),
    ];
    app.drawing.virtual_events(events, false);
    frame(&ctx, &mut app, 0.3, vec![]);
    if app.documents[0].snapshot() == original || app.documents[0].history_counts() != (1, 0) {
        return Err("User tool did not draw as one history entry".into());
    }
    let drawn = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != drawn
    {
        return Err("User tool undo/redo pixels diverged".into());
    }
    drawer_core::files::save(&mut app.documents[0], &root.join("user-tool-pixels.drawer"))
        .map_err(|e| e.to_string())?;
    let current = app.capture_workspace();
    let group = current.user_tools.group(&[aid, bid], None)?;
    let path = root.join("user-tool-group.json");
    group.save(&path, false).map_err(|e| e.to_string())?;
    let imported = Library::read(&path).map_err(|e| e.to_string())?;
    let mut draft = current.user_tools.clone();
    let map = draft.import_group(&imported)?;
    if map.iter().filter(|(old, new)| old != new).count() != map.len() {
        return Err("Group import reused existing IDs".into());
    }
    draft.delete_tools(&[aid])?;
    let next = draft.apply_to(&current)?;
    app.stage_workspace(next, app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    if app.selected != BrushTip::Pen
        || app.workspaces.store.current.user_tools.active.is_some()
        || app.panels.settings.brush_size != 12.0
        || app.command_reason(&acmd).is_none()
        || app
            .capture_workspace()
            .user_tools
            .missing_refs(&app.capture_workspace())
            .len()
            < 2
    {
        return Err(format!(
            "Active tool deletion fallback: selected={:?}, active={:?}, size={}, reason={:?}, missing={:?}",
            app.selected,
            app.workspaces.store.current.user_tools.active,
            app.panels.settings.brush_size,
            app.command_reason(&acmd),
            app.capture_workspace()
                .user_tools
                .missing_refs(&app.capture_workspace())
        ));
    }
    if app.documents[0].snapshot() != drawn
        || app.documents[0].history_counts() != (1, 0)
        || *app.materials.library != *materials
        || !app
            .workspaces
            .store
            .current
            .brushes
            .contains("Keep saved preset")
    {
        return Err("Tool management changed artwork/materials/saved presets".into());
    }
    let backup = Snapshot::read(&user.join("settings/workspace-before-apply.json"))
        .map_err(|e| e.to_string())?;
    app.stage_workspace(backup, app.workspaces.store.active.clone());
    app.apply_workspace(drawer_core::workspaces::Scope::ALL, None)?;
    if app.workspaces.store.current.user_tools.active != Some(aid)
        || app.command_reason(&acmd).is_some()
        || app.documents[0].snapshot() != drawn
    {
        return Err(
            "Settings restoration did not recover tool and command references together".into(),
        );
    }
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.capture_workspace().user_tools != app.capture_workspace().user_tools
        || *restarted.materials.library != *materials
    {
        return Err("User tools/materials not restored on restart".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_user_tools_commands_drawing_groups_deletion_backup_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
    #[test]
    fn failed_settings_write_restores_previous_tool_and_preserves_foreign_temp() {
        let root = crate::tests::TestDirectory::new();
        let ctx = egui::Context::default();
        let user = root.0.join("user");
        let mut app = Drawer::new(&ctx, None).with_storage(UserStorage::at(user.clone()).unwrap());
        app.sync_tool_properties();
        let mut next = app.capture_workspace();
        let preset = Preset::capture(&next, BrushTip::Pen, "Ink").unwrap();
        let id = next
            .user_tools
            .create_tool(1, preset, Property::ALL.to_vec())
            .unwrap();
        app.stage_workspace(next, app.workspaces.store.active.clone());
        app.apply_workspace(
            drawer_core::workspaces::Scope {
                other: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        app.drawing.mode = super::super::precision::Mode::Eyedropper;
        let before = app.capture_workspace();
        let file = user.join("settings/workspaces.json");
        let bytes = std::fs::read(&file).unwrap();
        let temp = file.with_extension("json.write.tmp");
        std::fs::write(&temp, b"foreign").unwrap();
        assert!(app.select_user_tool(id).is_err());
        assert!(app.capture_workspace() == before);
        assert_eq!(app.drawing.mode, super::super::precision::Mode::Eyedropper);
        assert_eq!(std::fs::read(file).unwrap(), bytes);
        assert_eq!(std::fs::read(&temp).unwrap(), b"foreign");
        std::fs::remove_file(temp).unwrap();
    }
}

#[cfg(test)]
mod slot_drag_tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        app: &mut Drawer,
        e: &mut Editor,
        mut events: Vec<egui::Event>,
        ctrl: bool,
        focused: bool,
    ) {
        events.insert(
            0,
            egui::Event::ModifiersChanged(egui::Modifiers {
                ctrl,
                ..Default::default()
            }),
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 1800.0),
                )),
                focused,
                events,
                ..Default::default()
            },
            |ui| {
                app.user_tools_ui(ui, e);
            },
        );
        output.textures_delta.clear();
    }
    fn press(pos: egui::Pos2, pressed: bool, ctrl: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers {
                ctrl,
                ..Default::default()
            },
        }
    }
    #[test]
    fn actual_slot_drag_copy_cancel_and_list_history_leave_picture_untouched() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(crate::tests::tiny_spec()).unwrap();
        app.sync_tool_properties();
        let before = app.documents[0].snapshot();
        let s = app.capture_workspace();
        let mut tools = s.user_tools.clone();
        let mut ids = vec![];
        for name in ["A", "B", "C"] {
            ids.push(
                tools
                    .create_tool(
                        1,
                        Preset::capture(&s, BrushTip::Pen, name).unwrap(),
                        Property::ALL.to_vec(),
                    )
                    .unwrap(),
            );
        }
        let mut e = Editor::new(tools.clone(), BrushTip::Pen);
        frame(&ctx, &mut app, &mut e, vec![], false, true);
        frame(&ctx, &mut app, &mut e, vec![], false, true);
        let start = e.rows.iter().find(|r| r.0 == ids[2]).unwrap().2.center();
        let target = e.rows.iter().find(|r| r.0 == ids[0]).unwrap().1.center();
        frame(
            &ctx,
            &mut app,
            &mut e,
            vec![egui::Event::PointerMoved(start), press(start, true, false)],
            false,
            true,
        );
        frame(
            &ctx,
            &mut app,
            &mut e,
            vec![egui::Event::PointerMoved(target)],
            false,
            true,
        );
        assert!(e.drag.is_some());
        assert!(e.draft == tools);
        frame(
            &ctx,
            &mut app,
            &mut e,
            vec![press(target, false, false)],
            false,
            true,
        );
        assert_eq!(e.draft.tools[0].id, ids[2]);
        assert_eq!(e.history.counts(), (1, 0));
        assert!(e.history.undo(&mut e.draft));
        assert!(e.draft == tools);
        assert!(e.history.redo(&mut e.draft));
        // Ctrl remains held until a valid drop: independent copy with a fresh ID.
        let start = e.rows.iter().find(|r| r.0 == ids[0]).unwrap().2.center();
        let target = e.rows.iter().find(|r| r.0 == ids[1]).unwrap().1.center();
        frame(
            &ctx,
            &mut app,
            &mut e,
            vec![egui::Event::PointerMoved(start), press(start, true, true)],
            true,
            true,
        );
        frame(
            &ctx,
            &mut app,
            &mut e,
            vec![egui::Event::PointerMoved(target)],
            true,
            true,
        );
        frame(
            &ctx,
            &mut app,
            &mut e,
            vec![press(target, false, true)],
            true,
            true,
        );
        assert_eq!(e.draft.tools.len(), 4);
        let copied = e.selected.iter().copied().next().unwrap();
        assert!(!ids.contains(&copied));
        let stable = e.draft.clone();
        for mode in 0..4 {
            frame(&ctx, &mut app, &mut e, vec![], true, true);
            let start = e.rows.iter().find(|r| r.0 == ids[0]).unwrap().2.center();
            let target = e.rows.iter().find(|r| r.0 == ids[1]).unwrap().1.center();
            frame(
                &ctx,
                &mut app,
                &mut e,
                vec![egui::Event::PointerMoved(start), press(start, true, true)],
                true,
                true,
            );
            frame(
                &ctx,
                &mut app,
                &mut e,
                vec![egui::Event::PointerMoved(target)],
                true,
                true,
            );
            assert!(e.drag.is_some());
            match mode {
                0 => frame(&ctx, &mut app, &mut e, vec![], false, true),
                1 => frame(
                    &ctx,
                    &mut app,
                    &mut e,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    true,
                    true,
                ),
                2 => frame(&ctx, &mut app, &mut e, vec![], true, false),
                _ => frame(
                    &ctx,
                    &mut app,
                    &mut e,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(1190.0, 1790.0)),
                        press(egui::pos2(1190.0, 1790.0), false, true),
                    ],
                    true,
                    true,
                ),
            }
            assert!(e.drag.is_none());
            assert!(e.draft == stable);
            frame(
                &ctx,
                &mut app,
                &mut e,
                vec![
                    press(target, false, false),
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                ],
                false,
                true,
            );
        }
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
