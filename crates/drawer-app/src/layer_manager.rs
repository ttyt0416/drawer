use super::*;
use drawer_core::layers::{self as core, Command, Kind, Target};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    tracked: std::collections::HashMap<u64, Tracked>,
    create: Option<egui::Rect>,
    duplicate: Option<egui::Rect>,
    rename: Option<egui::Rect>,
    delete: Option<egui::Rect>,
    movement: Option<egui::Rect>,
}
#[derive(Clone)]
pub(super) struct Tracked {
    revision: u64,
    key: Option<drawer_core::layer_search::Key>,
    shown: drawer_core::thumbnail::Target,
    rasters: Vec<u64>,
    vectors: Vec<u64>,
}
fn stamp(d: &Document, shown: drawer_core::thumbnail::Target) -> Tracked {
    Tracked {
        revision: d.revision,
        key: drawer_core::layer_catalog::from_thumbnail(d, shown),
        shown,
        rasters: (0..d.layers.len())
            .map(|n| d.scene.layer_ids.id(n))
            .collect(),
        vectors: d.scene.lines.layers.iter().map(|l| l.id).collect(),
    }
}
fn remap_tools(workspace: &mut drawer_core::workspaces::Snapshot, d: &Document, old: &Tracked) {
    let map: Vec<_> = old.rasters.iter().map(|id| core::index(d, *id)).collect();
    let vector_map: Vec<_> = old
        .vectors
        .iter()
        .map(|id| d.scene.lines.layers.iter().position(|l| l.id == *id))
        .collect();
    let bits = |old: [u64; 4], map: &[Option<usize>]| {
        let mut output = [0; 4];
        for (i, to) in map.iter().enumerate() {
            if drawer_core::fill_reference::selected(old, i)
                && let Some(to) = to
            {
                drawer_core::fill_reference::select(&mut output, *to, true);
            }
        }
        output
    };
    for fill in &mut workspace.input.fills {
        if map.iter().enumerate().any(|(i, n)| *n != Some(i)) {
            if let drawer_core::bucket::Reference::Layer(i) = fill.reference {
                fill.reference = map
                    .get(i as usize)
                    .copied()
                    .flatten()
                    .map_or(drawer_core::bucket::Reference::Active, |n| {
                        drawer_core::bucket::Reference::Layer(n as u16)
                    });
            }
            fill.selected = bits(fill.selected, &map);
        }
        if vector_map.iter().enumerate().any(|(i, n)| *n != Some(i)) {
            fill.selected_vectors = bits(fill.selected_vectors, &vector_map);
        }
    }
    let refs = &mut workspace.eyedropper.references;
    if refs.active(d) && map.iter().enumerate().any(|(i, n)| *n != Some(i)) {
        refs.include = refs
            .include
            .iter()
            .filter_map(|n| map.get(*n).copied().flatten())
            .collect();
        refs.exclude = refs
            .exclude
            .iter()
            .filter_map(|n| map.get(*n).copied().flatten())
            .collect();
    }
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    target: Target,
    name: String,
    kind: Kind,
    parent: Option<usize>,
    position: usize,
}
fn current(app: &Drawer, doc: &Document) -> Target {
    match app.panels.targets.get(&doc.id) {
        Some(drawer_core::thumbnail::Target::Vector(id)) => Target::Vector(*id),
        Some(drawer_core::thumbnail::Target::Raster(n)) if *n < doc.layers.len() => {
            Target::Raster(doc.scene.layer_ids.id(*n))
        }
        None => Target::Raster(doc.scene.layer_ids.id(0)),
        _ => Target::Raster(0),
    }
}
fn resolve(doc: &Document, target: Target) -> Option<drawer_core::thumbnail::Target> {
    match target {
        Target::Raster(id) => core::index(doc, id).map(drawer_core::thumbnail::Target::Raster),
        Target::Vector(id) => doc
            .scene
            .lines
            .layers
            .iter()
            .any(|l| l.id == id)
            .then_some(drawer_core::thumbnail::Target::Vector(id)),
    }
}
fn title(doc: &Document, t: Target) -> String {
    match t {
        Target::Raster(id) => core::index(doc, id)
            .map_or_else(|| "Missing raster".into(), |n| doc.layers[n].name.clone()),
        Target::Vector(id) => doc
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == id)
            .map_or_else(|| "Missing vector".into(), |l| l.name.clone()),
    }
}
impl Drawer {
    pub(super) fn finish_search_identity(&mut self, n: usize, before: &Tracked) {
        let d = &self.documents[n];
        remap_tools(&mut self.workspaces.store.current, d, before);
        self.layer_manager.tracked.remove(&d.id);
        self.sync_layer_identity();
    }
    pub(super) fn sync_layer_identity(&mut self) {
        self.layer_manager
            .tracked
            .retain(|id, _| self.documents.iter().any(|d| d.id == *id));
        for n in 0..self.documents.len() {
            let d = &self.documents[n];
            let shown = self
                .panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
            let invalid = drawer_core::layer_catalog::from_thumbnail(d, shown).is_none();
            if invalid {
                let target = self
                    .layer_manager
                    .tracked
                    .get(&d.id)
                    .and_then(|old| {
                        old.key
                            .and_then(|k| drawer_core::layer_catalog::thumbnail(d, k))
                    })
                    .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
                self.panels.targets.insert(d.id, target);
            }
            if let Some(old) = self.layer_manager.tracked.get(&d.id).cloned()
                && old.revision != d.revision
            {
                if self.active == Some(n) {
                    remap_tools(&mut self.workspaces.store.current, d, &old);
                }
                if shown == old.shown {
                    let value = old
                        .key
                        .and_then(|k| drawer_core::layer_catalog::thumbnail(d, k))
                        .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
                    self.panels.targets.insert(d.id, value);
                }
            }
            let shown = self
                .panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
            self.layer_manager.tracked.insert(d.id, stamp(d, shown));
        }
    }
    pub(super) fn begin_layer_manager(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = current(self, d);
        let parent = if let Some(drawer_core::thumbnail::Target::Folder(n)) =
            self.panels.targets.get(&d.id)
        {
            Some(*n)
        } else {
            match target {
                Target::Raster(id) => {
                    core::index(d, id).and_then(|n| core::parent(&d.scene.paper, n))
                }
                _ => None,
            }
        };
        self.layer_manager.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            name: title(d, target),
            kind: Kind::Raster,
            parent,
            position: 0,
        }));
        Ok(())
    }
    pub(super) fn layer_command(&mut self, command: Command) -> Result<(), String> {
        self.finish_stroke();
        let n = self.active.ok_or("Open a page first")?;
        let old = stamp(
            &self.documents[n],
            self.panels
                .targets
                .get(&self.documents[n].id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        let prepared = core::prepare(&self.documents[n], command)?;
        if prepared
            .state
            .memory_bytes()
            .saturating_mul(2)
            .saturating_add(used)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Layer change exceeds workspace memory".into());
        }
        let selected = prepared.selected;
        self.documents[n].commit_state(prepared.state)?;
        let d = &self.documents[n];
        remap_tools(&mut self.workspaces.store.current, d, &old);
        let target = resolve(d, selected).unwrap_or(drawer_core::thumbnail::Target::Raster(0));
        self.panels.targets.insert(d.id, target);
        self.layer_manager.tracked.insert(d.id, stamp(d, target));
        if let Some(e) = self.layer_manager.draft.as_mut().filter(|e| e.id == d.id) {
            e.revision = d.revision;
            e.target = selected;
            e.name = title(d, selected);
            e.parent = match selected {
                Target::Raster(id) => {
                    core::index(d, id).and_then(|n| core::parent(&d.scene.paper, n))
                }
                _ => None,
            };
        }
        Ok(())
    }
    pub(super) fn layer_quick_command(&mut self, command: &str) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = current(self, d);
        if target == Target::Raster(0) && matches!(command, "duplicate-layer" | "delete-layer") {
            if command == "duplicate-layer" {
                return Err("Select a raster or vector layer to duplicate".into());
            }
            let key = self
                .panels
                .targets
                .get(&d.id)
                .copied()
                .and_then(|t| drawer_core::layer_catalog::from_thumbnail(d, t))
                .ok_or("Select a layer")?;
            let selection = drawer_core::layer_search::Selection {
                keys: [key].into_iter().collect(),
                primary: Some(key),
                anchor: Some(key),
            };
            return self.search_edit(
                &Default::default(),
                &selection,
                drawer_core::search_edit::Edit::Delete,
            );
        }
        let command = match command {
            "new-raster-layer" => Command::Create {
                kind: Kind::Raster,
                name: "Raster layer".into(),
                parent: if let Some(drawer_core::thumbnail::Target::Folder(n)) =
                    self.panels.targets.get(&d.id)
                {
                    Some(*n)
                } else {
                    match target {
                        Target::Raster(id) => {
                            core::index(d, id).and_then(|n| core::parent(&d.scene.paper, n))
                        }
                        _ => None,
                    }
                },
            },
            "new-vector-layer" => Command::Create {
                kind: Kind::Vector,
                name: "Vector layer".into(),
                parent: None,
            },
            "duplicate-layer" => Command::Duplicate(target),
            "delete-layer" => Command::Delete(target),
            _ => return Err("Unknown layer command".into()),
        };
        self.layer_command(command)
    }
    pub(super) fn layer_manager_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_manager.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = d.revision == e.revision && d.current_frame == e.frame;
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut command = None;
        egui::Window::new("Manage local layers").default_width(520.0).min_width(520.0).default_pos(egui::pos2(70.0,40.0)).resizable(false).collapsible(false).open(&mut open).show(ctx,|ui|{
            if !fresh {ui.label("Page/frame changed; cancel and reopen layers.");return;}
            let mut change=None;
            egui::ComboBox::from_id_salt("managed-layer").selected_text(format!("{} · {:?}",title(d,e.target),e.target)).show_ui(ui,|ui|{
                for n in core::order(d){let t=Target::Raster(d.scene.layer_ids.id(n));if ui.selectable_value(&mut e.target,t,format!("{} · raster {}",d.layers[n].name,d.scene.layer_ids.id(n))).changed(){change=Some(t);}}
                for l in &d.scene.lines.layers{let t=Target::Vector(l.id);if ui.selectable_value(&mut e.target,t,format!("{} · vector {}",l.name,l.id)).changed(){change=Some(t);}}
            });
            if let Some(t)=change{e.name=title(d,t);e.parent=match t{Target::Raster(id)=>core::index(d,id).and_then(|n|core::parent(&d.scene.paper,n)),_=>None};}
            super::edit::text(ui,&mut e.name,"managed-layer-name");
            ui.horizontal(|ui|{ui.selectable_value(&mut e.kind,Kind::Raster,"New raster");ui.selectable_value(&mut e.kind,Kind::Vector,"New vector");});
            egui::ComboBox::from_id_salt("managed-layer-parent").selected_text(e.parent.map_or("Root".into(),|n|format!("Folder {}",n+1))).show_ui(ui,|ui|{ui.selectable_value(&mut e.parent,None,"Root");for(n,f)in d.scene.paper.folders.iter().enumerate().filter(|(n,_)|d.scene.paper.folder_active(*n)){ui.selectable_value(&mut e.parent,Some(n),&f.name);}});
            ui.horizontal(|ui|{
                let r=ui.button("Create layer");self.layer_manager.create=Some(r.rect);if r.clicked(){command=Some(Command::Create {kind:e.kind,name:e.name.clone(),parent:if e.kind==Kind::Vector{None}else{e.parent}});}
                let r=ui.button("Rename layer");self.layer_manager.rename=Some(r.rect);if r.clicked(){command=Some(Command::Rename {target:e.target,name:e.name.clone()});}
                let r=ui.button("Duplicate layer");self.layer_manager.duplicate=Some(r.rect);if r.clicked(){command=Some(Command::Duplicate(e.target));}
                let allowed=!matches!(e.target,Target::Raster(_) if d.layers.len()==1);let r=ui.add_enabled(allowed,egui::Button::new("Delete layer"));self.layer_manager.delete=Some(r.rect);if r.clicked(){command=Some(Command::Delete(e.target));}
            });
            let length=match e.target{Target::Raster(id)=>{let same=core::index(d,id).is_some_and(|i|core::parent(&d.scene.paper,i)==e.parent);e.parent.map_or(d.scene.paper.root_nodes(d.layers.len()).len(),|n|d.scene.paper.folders.get(n).map_or(0,|f|f.children.len())).saturating_sub(usize::from(same))},Target::Vector(_)=>d.scene.lines.layers.len().saturating_sub(1)};
            ui.add(egui::Slider::new(&mut e.position,0..=length).text("Destination slot after removal (bottom = 0)"));
            let r=ui.button("Move to folder / slot");self.layer_manager.movement=Some(r.rect);if r.clicked(){command=Some(Command::Move {target:e.target,parent:if matches!(e.target,Target::Vector(_)){None}else{e.parent},position:e.position});}
            ui.label("Each button applies one Undo. Equal names are allowed; identities remain separate. The last raster layer cannot be deleted. Duplicates keep their pixels, paths and properties. Raster folders and the vector stack have separate composition order.");
        });
        if open {
            self.layer_manager.draft = Some(e);
        }
        if let Some(command) = command
            && let Err(err) = self.layer_command(command)
        {
            self.status = Some(err);
        }
    }
}
pub(super) fn search_stamp(d: &Document) -> Tracked {
    stamp(d, drawer_core::thumbnail::Target::Raster(0))
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layers_verify::verify(root)?;
    super::image_import_dialog::layer_smoke(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            step(
                app,
                time,
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
        step(app, time, vec![]);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    app.execute_command(&ctx, "manage-layers");
    app.layer_manager
        .draft
        .as_mut()
        .ok_or("Layer manager command missing")?
        .name = "Same name".into();
    for action in ["create", "duplicate", "rename", "move", "delete"] {
        if action == "rename" {
            app.layer_manager
                .draft
                .as_mut()
                .ok_or("Layer draft lost")?
                .name = "Changed".into();
        }
        if action == "move" {
            let e = app.layer_manager.draft.as_mut().unwrap();
            e.parent = None;
            e.position = 0;
        }
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let panel = &app.layer_manager;
        let r = match action {
            "create" => panel.create,
            "duplicate" => panel.duplicate,
            "rename" => panel.rename,
            "move" => panel.movement,
            _ => panel.delete,
        }
        .ok_or("Layer production button missing")?;
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts().0;
        click(&mut app, &mut time, r);
        if app.documents[0].history_counts().0 != count + 1 {
            return Err(format!("Layer {action} button: {:?} {r:?}", app.status));
        }
        if action == "move" && core::order(&app.documents[0])[0] != 2 {
            return Err("Layer production order mismatch".into());
        }
        if action == "delete" {
            let cfg = &mut app.workspaces.store.current.input.fills[0];
            cfg.reference = drawer_core::bucket::Reference::Layer(1);
            drawer_core::fill_reference::select(&mut cfg.selected, 1, true);
            app.layer_manager.draft = None;
            app.documents[0].undo();
            app.sync_layer_identity();
            if app.documents[0].snapshot() != before {
                return Err("Layer deletion UI Undo".into());
            }
            app.documents[0].redo();
            app.sync_layer_identity();
            app.execute_command(&ctx, "manage-layers");
        }
    }
    let file = root.join("layers-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
    {
        return Err("Layer UI native reopen".into());
    }
    app.layer_manager.draft = None;
    while app.documents[0].undo() {}
    app.sync_layer_identity();
    if app.documents[0].snapshot() != original {
        return Err("Layer UI history did not restore original document".into());
    }
    app.execute_command(&ctx, "new-raster-layer");
    app.execute_command(&ctx, "new-raster-layer");
    let third = app.documents[0].scene.layer_ids.id(2);
    app.workspaces.store.current.input.fills[0].reference =
        drawer_core::bucket::Reference::Layer(2);
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(2),
    );
    app.sync_layer_identity();
    app.layer_command(Command::Delete(Target::Raster(1)))?;
    if app.workspaces.store.current.input.fills[0].reference
        != drawer_core::bucket::Reference::Layer(1)
    {
        return Err("Tool reference did not follow stable owner on delete".into());
    }
    let chosen = current(&app, &app.documents[0]);
    app.documents[0].undo();
    app.sync_layer_identity();
    if current(&app, &app.documents[0]) != chosen {
        return Err("Selected owner changed after layer Undo".into());
    }
    if app.workspaces.store.current.input.fills[0].reference
        != drawer_core::bucket::Reference::Layer(2)
    {
        return Err("Tool reference did not follow stable owner on Undo".into());
    }
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(2),
    );
    app.sync_layer_identity();
    if current(&app, &app.documents[0]) != Target::Raster(third) {
        return Err("Active raster identity changed after Undo".into());
    }
    app.execute_command(&ctx, "manage-layers");
    let before = app.documents[0].snapshot();
    step(
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.layer_manager.draft.is_some() || app.documents[0].snapshot() != before {
        return Err("Layer manager Escape changed page".into());
    }
    app.execute_command(&ctx, "new-vector-layer");
    app.execute_command(&ctx, "duplicate-layer");
    if app.documents[0].scene.lines.layers.len() != 2 {
        return Err(format!("Vector layer commands failed: {:?}", app.status));
    }
    app.execute_command(&ctx, "manage-layers");
    app.documents.clear();
    app.active = None;
    app.layer_manager_window(&ctx);
    if app.layer_manager.draft.is_some() {
        return Err("Closed layer document retained a draft".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn layer_buttons_ids_references_undo_and_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
