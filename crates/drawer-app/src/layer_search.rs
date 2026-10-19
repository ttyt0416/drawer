use super::*;
use drawer_core::{
    layer_catalog as catalog,
    layer_properties::Patch,
    layer_search::{self as core, Condition, Context, Kinds, Scope, Selection, Settings, Sort},
    search_edit::{self, Edit},
};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    selections: std::collections::HashMap<u64, Selection>,
    name: String,
    tag: String,

    rows: std::collections::HashMap<core::Key, egui::Rect>,

    buttons: std::collections::HashMap<&'static str, egui::Rect>,
}
impl Drawer {
    pub(super) fn search_edit(
        &mut self,
        settings: &Settings,
        selected: &Selection,
        edit: Edit,
    ) -> Result<(), String> {
        self.finish_stroke();
        let n = self.active.ok_or("Open a page first")?;
        let before = super::layer_manager::search_stamp(&self.documents[n]);
        let result = search_edit::prepare(&self.documents[n], settings, selected, edit)?;
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(result.state.memory_bytes().saturating_mul(2))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Search edit exceeds workspace memory".into());
        }
        self.documents[n].commit_state(result.state)?;
        self.finish_search_identity(n, &before);
        let d = &self.documents[n];
        if let Some(k) = selected.primary.and_then(|k| catalog::thumbnail(d, k)) {
            self.panels.targets.insert(d.id, k);
        } else {
            self.panels
                .targets
                .insert(d.id, drawer_core::thumbnail::Target::Raster(0));
        }
        Ok(())
    }
    pub(super) fn layer_search_window(&mut self, ctx: &egui::Context) {
        if !self.layer_search.open {
            return;
        }
        let Some(n) = self.active else {
            return;
        };
        let d = &self.documents[n];
        let id = d.id;
        let mut panel = std::mem::take(&mut self.layer_search);
        panel.rows.clear();
        panel.buttons.clear();
        panel
            .selections
            .retain(|id, _| self.documents.iter().any(|d| d.id == *id));
        let mut selection = panel.selections.remove(&id).unwrap_or_default();
        let rows = match catalog::rows(d) {
            Ok(r) => r,
            Err(e) => {
                self.status = Some(e);
                self.layer_search = panel;
                return;
            }
        };
        selection.retain(&rows);
        selection.primary = catalog::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        let mut settings = self.workspaces.store.current.layer_search.clone();
        let mut open = true;
        let mut edit = None;
        let mut picked = None;
        egui::Window::new("Search local layers").open(&mut open).default_width(620.).default_pos(egui::pos2(20.,15.)).show(ctx,|ui|{
            ui.horizontal(|ui|{ui.label("Name (literal)");
ui.add(egui::TextEdit::singleline(&mut settings.name).char_limit(256));
ui.label("Tag");
ui.add(egui::TextEdit::singleline(&mut settings.tag).char_limit(64));});
            ui.horizontal(|ui|{if ui.button("All types").clicked(){settings.kinds=Kinds::All;}
if ui.button("Clear types").clicked(){settings.kinds=Kinds::Some(Default::default());}egui::ComboBox::from_id_salt("layer-search-sort").selected_text(format!("{:?}",settings.sort)).show_ui(ui,|ui|{
for s in [Sort::Tree,Sort::Name,Sort::Kind]{ui.selectable_value(&mut settings.sort,s,format!("{s:?}"));}});});
            ui.collapsing("Layer types and attached features (any selected)",|ui|{ui.horizontal_wrapped(|ui|{
for kind in core::Kind::ALL{let mut enabled=match &settings.kinds{Kinds::All=>true,Kinds::Some(k)=>k.contains(kind)};if ui.checkbox(&mut enabled,kind.label()).changed(){let mut kinds=match &settings.kinds{Kinds::All=>core::Kind::ALL.iter().copied().collect(),Kinds::Some(k)=>k.clone()};if enabled{kinds.insert(*kind);}else{kinds.remove(kind);}settings.kinds=Kinds::Some(kinds);}}});});
            ui.collapsing("States (all conditions must match)",|ui|{egui::Grid::new("layer-search-states").num_columns(4).show(ui,|ui|{for(i,label)in core::STATES.iter().enumerate(){ui.label(*label);
for c in Condition::ALL{ui.radio_value(&mut settings.states[i],c,format!("{c:?}"));}ui.end_row();}});});
            egui::ComboBox::from_label("Current folder scope").selected_text(format!("{:?}",settings.scope)).show_ui(ui,|ui|{ui.selectable_value(&mut settings.scope,Scope::All,"All scopes");
for c in [Context::Folder,Context::Panel,Context::Animation]{
for s in [Scope::Inside(c),Scope::Outside(c)]{ui.selectable_value(&mut settings.scope,s,format!("{s:?}"));}}});
            ui.small("Folder context follows this page's active selection. No matching current folder gives no results, including outside scope.");
            let results=match core::query(&rows,&settings,selection.primary,d.scene.catalog.expression){Ok(r)=>r,Err(e)=>{ui.label(e);return;}};
            let visible=selection.visible(&results).len();
            ui.label(format!("{} results · {} selected here · {} selected outside results",results.len(),visible,selection.keys.len()-visible));
            ui.horizontal(|ui|{if ui.button("Select all results").clicked(){selection.keys.extend(results.iter().copied());}
if ui.button("Clear selection").clicked(){selection.keys.clear();selection.anchor=None;}});
            let map=rows.iter().map(|r|(r.key,r)).collect::<std::collections::BTreeMap<_,_>>();
            egui::ScrollArea::vertical().id_salt("layer-search-results").max_height(200.).show_rows(ui,24.,results.len(),|ui,range|{
for i in range{let key=results[i];let row=map[&key];let flags=if row.states[1]{" · locked"}else if !row.states[0]{" · hidden"}else{""};let r=ui.selectable_label(selection.keys.contains(&key),format!("{} · {}{flags}",row.name,row.kind.label()));
                {panel.rows.insert(key,r.rect);}
                if r.clicked(){let modifiers=ui.input(|i|i.modifiers);selection.click(&results,key,modifiers.command||modifiers.ctrl,modifiers.shift);picked=Some(key);}
            }});
            ui.add_enabled_ui(visible>0,|ui|{
                ui.horizontal_wrapped(|ui|{for(label,value)in [("Show",false),("Hide",true)]{let r=ui.button(label);{panel.buttons.insert(label,r.rect);}
if r.clicked(){edit=Some(Edit::Properties(Patch{hidden:Some(value),..Default::default()}));}}for(label,value)in [("Lock",true),("Unlock",false)]{if ui.button(label).clicked(){edit=Some(Edit::Properties(Patch{locked:Some(value),..Default::default()}));}}for(label,value)in [("Protect alpha",true),("Unprotect alpha",false)]{if ui.button(label).clicked(){edit=Some(Edit::Alpha(value));}}});
                ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut panel.name).char_limit(64).desired_width(160.));if ui.button("Rename results").clicked(){edit=Some(Edit::Rename(panel.name.clone()));}});
                ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut panel.tag).char_limit(32).desired_width(160.));if ui.button("Add tag").clicked(){edit=Some(Edit::Tag{value:panel.tag.clone(),remove:false});}
if ui.button("Remove tag").clicked(){edit=Some(Edit::Tag{value:panel.tag.clone(),remove:true});}let r=ui.button("Delete selected results");{panel.buttons.insert("Delete",r.rect);}
if r.clicked(){edit=Some(Edit::Delete);}});
            });
            ui.small("Edits affect selected results currently shown. Ctrl toggles; Shift selects a range. Search, sorting and selection leave drawing history unchanged.");
        });
        if let Some(k) = picked.and_then(|k| catalog::thumbnail(d, k)) {
            self.panels.targets.insert(id, k);
        }
        if let Some(edit) = edit
            && let Err(e) = self.search_edit(&settings, &selection, edit)
        {
            self.status = Some(e);
        }
        self.workspaces.store.current.layer_search = settings;
        panel.open = open;
        panel.selections.insert(id, selection);
        self.layer_search = panel;
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::search_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for name in ["wanted", "excluded"] {
        let state = drawer_core::layers::prepare(
            &app.documents[0],
            drawer_core::layers::Command::Create {
                kind: drawer_core::layers::Kind::Raster,
                name: name.into(),
                parent: None,
            },
        )?
        .state;
        app.documents[0].commit_state(state)?;
    }
    let id = app.documents[0].id;
    let a = core::Key::Layer(drawer_core::layer_properties::Target::Raster(2));
    let b = core::Key::Layer(drawer_core::layer_properties::Target::Raster(3));
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    app.execute_command(&ctx, "layer-search");
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let click = |app: &mut Drawer, time: &mut f64, rect: egui::Rect, modifiers: egui::Modifiers| {
        for pressed in [true, false] {
            step(
                app,
                time,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
            );
        }
    };
    let r = *app
        .layer_search
        .rows
        .get(&a)
        .ok_or("Search result row missing")?;
    click(&mut app, &mut time, r, Default::default());
    let r = *app
        .layer_search
        .rows
        .get(&b)
        .ok_or("Second search result missing")?;
    click(
        &mut app,
        &mut time,
        r,
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    );
    if app.layer_search.selections[&id].keys.len() != 2
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != history
    {
        return Err("Search result multiple selection changed document/history".into());
    }
    app.workspaces.store.current.layer_search.name = "wanted".into();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = *app
        .layer_search
        .buttons
        .get("Hide")
        .ok_or("Search Hide button missing")?;
    click(&mut app, &mut time, r, Default::default());
    if !drawer_core::layer_properties::properties(
        &app.documents[0],
        drawer_core::layer_properties::Target::Raster(2),
    )
    .hidden
        || drawer_core::layer_properties::properties(
            &app.documents[0],
            drawer_core::layer_properties::Target::Raster(3),
        )
        .hidden
    {
        return Err(format!(
            "Search UI edited filtered-out results: {:?}",
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Search button is not one Undo".into());
    }
    let settings = app.capture_workspace();
    if settings.layer_search.name != "wanted" {
        return Err("Search settings not captured".into());
    }
    let mut restarted = Drawer::new(&ctx, None);
    let storage = UserStorage::at(root.join("layer-search-settings")).map_err(|e| e.to_string())?;
    app.workspaces.store.current = settings.clone();
    app.workspaces
        .store
        .save(&storage)
        .map_err(|e| e.to_string())?;
    restarted.workspaces.store = drawer_core::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Search settings missing on disk")?;
    restarted.restore_workspace(&settings);
    if restarted.capture_workspace().layer_search != settings.layer_search {
        return Err("Search settings restart".into());
    }
    app.layer_search.open = false;
    let mut scene = app.documents[0].scene.clone();
    scene.objects.push(drawer_core::scene::PageObject::Line {
        start: [2., 5.],
        end: [11., 5.],
        width_px: 2.,
        color: [255, 0, 0, 255],
    });
    app.documents[0].edit_scene(scene)?;
    app.documents[0].view.zoom = 3.;
    app.documents[0].view.angle = 90.;
    app.execute_command(&ctx, "Move");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    use drawer_core::pointer_input::Phase;
    for (point, phase, modifiers) in [
        (
            [6., 5.],
            Phase::Down,
            egui::Modifiers {
                alt: true,
                ..Default::default()
            },
        ),
        ([8., 7.], Phase::Move, Default::default()),
        ([8., 7.], Phase::Up, Default::default()),
    ] {
        let event = super::precision::pointer(&ctx, &app, point, phase);
        app.drawing.virtual_events(vec![event], false);
        step(
            &mut app,
            &mut time,
            vec![egui::Event::ModifiersChanged(modifiers)],
        );
    }
    if app.documents[0].scene.objects[0].bounds().min != [3., 6.]
        || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!(
            "Picked page object drag failed: {:?}, {:?}",
            app.documents[0].scene.objects[0].bounds(),
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Picked object drag Undo".into());
    }
    let selected = Selection {
        keys: [a, b].into_iter().collect(),
        primary: catalog::from_thumbnail(&app.documents[0], app.panels.targets[&id]),
        anchor: Some(a),
    };
    drawer_core::fill_reference::select(
        &mut app.workspaces.store.current.input.fills[0].selected,
        2,
        true,
    );
    app.search_edit(
        &Settings {
            name: "wanted".into(),
            ..Default::default()
        },
        &selected,
        Edit::Delete,
    )?;
    if !drawer_core::fill_reference::selected(
        app.workspaces.store.current.input.fills[0].selected,
        1,
    ) || drawer_core::fill_reference::selected(
        app.workspaces.store.current.input.fills[0].selected,
        2,
    ) {
        return Err("Search deletion lost local fill reference identity".into());
    }
    let source = app.panels.targets[&id];
    if !app.documents[0].undo() {
        return Err("Search deletion Undo missing".into());
    }
    step(&mut app, &mut time, vec![]);
    if app.panels.targets[&id] != source
        || !drawer_core::fill_reference::selected(
            app.workspaces.store.current.input.fills[0].selected,
            2,
        )
        || drawer_core::fill_reference::selected(
            app.workspaces.store.current.input.fills[0].selected,
            1,
        )
    {
        return Err("Search deletion Undo lost active source or local fill reference".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn real_search_rows_multi_select_filtered_bulk_and_picked_drag() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
