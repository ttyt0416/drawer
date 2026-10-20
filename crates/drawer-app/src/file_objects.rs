use super::*;
use drawer_core::{
    file_objects as core, file_sources as sources, layer_catalog as catalog, layer_search::Key,
    sizing::DocumentState,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    new: Option<egui::Rect>,
    refresh: Option<egui::Rect>,
    refresh_all: Option<egui::Rect>,
    inspect: Option<egui::Rect>,
    acknowledge: Option<egui::Rect>,
    convert: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    source_smoke(root)?;
    drawer_core::file_objects_verify::verify(root)?;
    let path = drawer_core::file_objects_verify::source(
        root,
        "ui-file-source.drawer",
        [220, 60, 30, 128],
    )?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.;
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
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    app.execute_command(&ctx, "linked-file-layer");
    app.file_objects
        .draft
        .as_mut()
        .ok_or("File command missing")?
        .path = path.to_string_lossy().into_owned();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.file_objects.new.ok_or("File preview button missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].snapshot() != original
        || app
            .file_objects
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!(
            "File preview mutated page or failed: {:?}",
            app.status
        ));
    }
    let r = app.file_objects.apply.ok_or("File apply missing")?;
    click(&mut app, &mut time, r);
    if app.file_objects.draft.is_some()
        || drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5; 2])
            != [220, 60, 30, 128]
    {
        return Err(format!("File apply failed: {:?}", app.status));
    }
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
    {
        return Err("File link Undo/Redo".into());
    }
    let original = app.documents[0].snapshot();
    app.execute_command(&ctx, "linked-file-layer");
    app.open_file_object_source(&ctx, false)?;
    if app.documents.len() != 2 || app.active != Some(1) || app.documents[0].snapshot() != original
    {
        return Err("Linked native source did not open separately".into());
    }
    app.documents[1].layers[0].pixels[0] = [30, 90, 150, 255];
    drawer_core::files::save(&mut app.documents[1], &path).map_err(|e| e.to_string())?;
    app.select_tab(0);
    if drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5; 2]) != [220, 60, 30, 128] {
        return Err("Opening source automatically refreshed its link".into());
    }
    app.execute_command(&ctx, "linked-file-layer");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.file_objects.refresh.ok_or("File refresh missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.file_objects.apply.unwrap();
    click(&mut app, &mut time, r);
    if drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5; 2]) != [30, 90, 150, 255] {
        return Err(format!("File manual refresh failed: {:?}", app.status));
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    app.execute_command(&ctx, "linked-file-layer");
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.file_objects.refresh.unwrap();
    click(&mut app, &mut time, r);
    if app
        .file_objects
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_some())
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err("Missing source refresh changed cache/history".into());
    }
    let relink = drawer_core::file_objects_verify::source(
        root,
        "ui-file-relink.drawer",
        [40, 100, 160, 128],
    )?;
    app.file_objects.draft.as_mut().unwrap().path = relink.to_string_lossy().into_owned();
    step(&mut app, &mut time, vec![]);
    let r = app.file_objects.refresh.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.file_objects.apply.unwrap();
    click(&mut app, &mut time, r);
    if drawer_core::render::Renderer::new(&app.documents[0]).pixel([0.5; 2]) != [40, 100, 160, 128]
    {
        return Err(format!("File relink failed: {:?}", app.status));
    }
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    app.documents[0].layers[0].pixels[27] = [20, 80, 140, 128];
    let original = app.documents[0].snapshot();
    let converted = root.join("ui-file-converted.drawer");
    app.execute_command(&ctx, "linked-file-layer");
    app.file_objects.draft.as_mut().unwrap().path = converted.to_string_lossy().into_owned();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .file_objects
        .convert
        .ok_or("File conversion button missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if converted.exists()
        || app.documents[0].snapshot() != original
        || app
            .file_objects
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!(
            "File conversion wrote before Apply or failed: {:?}",
            app.status
        ));
    }
    let r = app.file_objects.apply.unwrap();
    click(&mut app, &mut time, r);
    if !converted.is_file()
        || app.file_objects.draft.is_some()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !converted.is_file()
    {
        return Err(format!(
            "File conversion Apply/Undo/external preservation: {:?}",
            app.status
        ));
    }
    app.execute_command(&ctx, "linked-file-layer");
    app.file_objects.draft.as_mut().unwrap().path = converted.to_string_lossy().into_owned();
    app.preview_file_object(&ctx, Action::Convert, None)?;
    if app.apply_file_object().is_ok() || app.documents[0].snapshot() != original {
        return Err("Existing source overwritten without checkbox approval".into());
    }
    app.documents[0].current_frame += 1;
    if app.apply_file_object().is_ok() {
        return Err("Stale file frame accepted".into());
    }
    app.documents[0].current_frame -= 1;
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
    if app.file_objects.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("File Escape changed source".into());
    }
    let native = root.join("ui-file-cache.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    std::fs::remove_file(relink).map_err(|e| e.to_string())?;
    let reopened =
        drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    if reopened.snapshot() != app.documents[0].snapshot() {
        return Err("File UI cache did not reopen without source".into());
    }
    Ok(())
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    key: Option<Key>,
    path: String,
    overwrite: bool,
    selection: sources::Selection,
    metadata: Option<sources::Metadata>,
    source_stamps: Vec<sources::Stamp>,
    notes: Vec<String>,
    review_all: bool,
    acknowledged: bool,
    stamp: Option<(u64, std::time::SystemTime)>,
    prepared: Option<Box<Prepared>>,
    image: Option<egui::TextureHandle>,
}
enum Prepared {
    Change { state: Box<DocumentState>, key: Key },
    Convert(Box<core::Conversion>),
}
impl Prepared {
    fn state(&self) -> &DocumentState {
        match self {
            Self::Change { state, .. } => state,
            Self::Convert(c) => &c.state,
        }
    }
    fn key(&self) -> Key {
        match self {
            Self::Change { key, .. } => *key,
            Self::Convert(c) => c.target,
        }
    }
}
#[derive(Clone, Copy)]
enum Action {
    New,
    Refresh,
    RefreshAll,
    Inspect,
    Convert,
    Clipboard,
    Restore,
}
fn stamp(path: &std::path::Path) -> Option<(u64, std::time::SystemTime)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}
fn source_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::file_sources_verify::verify(root)?;
    let path = drawer_core::file_sources_verify::source(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.;
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
    };
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    app.execute_command(&ctx, "linked-file-layer");
    app.file_objects
        .draft
        .as_mut()
        .ok_or("Source editor missing")?
        .path = path.to_string_lossy().into_owned();
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    let r = app.file_objects.inspect.ok_or("Source inspect missing")?;
    click(&mut app, &mut time, r);
    if app
        .file_objects
        .draft
        .as_ref()
        .unwrap()
        .metadata
        .as_ref()
        .is_none_or(|m| m.frames != 2 || !m.timeline || m.comps.len() != 1)
    {
        return Err(format!("Source choice inventory failed: {:?}", app.status));
    }
    let selection = sources::Selection {
        content: sources::Content::Layer(Key::Layer(
            drawer_core::layer_properties::Target::Raster(1),
        )),
        comp: Some(1),
        frame: 1,
    };
    app.file_objects.draft.as_mut().unwrap().selection = selection.clone();
    let before = app.documents[0].snapshot();
    let r = app.file_objects.new.unwrap();
    click(&mut app, &mut time, r);
    if app.file_objects.draft.as_ref().unwrap().prepared.is_none()
        || app.documents[0].snapshot() != before
    {
        return Err(format!("Selected source preview failed: {:?}", app.status));
    }
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    let r = app.file_objects.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.file_objects.draft.is_some()
        || app.documents[0]
            .scene
            .file_objects
            .entries
            .first()
            .is_none_or(|l| l.selection != selection)
    {
        return Err(format!("Selected source apply failed: {:?}", app.status));
    }
    let missing = drawer_core::file_objects_verify::source(
        root,
        "ui-missing-refresh.drawer",
        [40, 80, 120, 255],
    )?;
    let (state, _, _) = sources::create(
        &app.documents[0],
        &missing,
        "Keep missing cache",
        Default::default(),
    )?;
    app.documents[0].commit_state(state)?;
    std::fs::remove_file(&missing).map_err(|e| e.to_string())?;
    let mut external =
        drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    external.layers[0].pixels.fill([70, 90, 110, 255]);
    drawer_core::files::save(&mut external, &path).map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    app.execute_command(&ctx, "refresh-file-layers");
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    let r = app.file_objects.refresh_all.ok_or("Refresh all missing")?;
    click(&mut app, &mut time, r);
    let e = app
        .file_objects
        .draft
        .as_ref()
        .ok_or("Refresh all editor closed")?;
    if e.prepared.is_none()
        || !e.review_all
        || !e.notes.iter().any(|s| s.contains("1 failures"))
        || app.documents[0].snapshot() != before
    {
        return Err(format!("Partial refresh UI missing: {:?}", app.status));
    }
    if app.apply_file_object().is_ok() {
        return Err("Refresh batch bypassed confirmation".into());
    }
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    let r = app
        .file_objects
        .acknowledge
        .ok_or("Batch confirmation missing")?;
    click(&mut app, &mut time, r);
    let r = app.file_objects.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.file_objects.draft.is_some() || app.documents[0].history_counts().0 != count.0 + 1 {
        return Err(format!("Refresh batch apply failed: {:?}", app.status));
    }
    let after = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Refresh batch Undo/Redo failed".into());
    }
    app.begin_file_objects()?;
    app.preview_file_object(&ctx, Action::RefreshAll, None)?;
    app.file_objects.draft.as_mut().unwrap().acknowledged = true;
    external.layers[0].pixels.fill([130, 150, 170, 255]);
    drawer_core::files::save(&mut external, &path).map_err(|e| e.to_string())?;
    if app.apply_file_object().is_ok() || app.documents[0].snapshot() != after {
        return Err("Changed source applied after review".into());
    }
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
    if app.file_objects.draft.is_some() || app.documents[0].snapshot() != after {
        return Err("Source batch Escape changed document".into());
    }
    Ok(())
}
impl Drawer {
    pub(super) fn begin_file_objects(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let key = catalog::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        let path = key
            .and_then(|k| d.scene.file_objects.get(k))
            .map_or(String::new(), |l| l.path.to_string_lossy().into_owned());
        self.file_objects.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            key,
            path,
            overwrite: false,
            selection: key
                .and_then(|k| d.scene.file_objects.get(k))
                .map_or_else(Default::default, |l| l.selection.clone()),
            metadata: None,
            source_stamps: vec![],
            notes: vec![],
            review_all: false,
            acknowledged: false,
            stamp: None,
            prepared: None,
            image: None,
        }));
        Ok(())
    }
    fn preview_file_object(
        &mut self,
        ctx: &egui::Context,
        action: Action,
        packet: Option<drawer_core::paste::ClipboardImage>,
    ) -> Result<(), String> {
        let e = self
            .file_objects
            .draft
            .as_ref()
            .ok_or("File layer editor closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen linked files".into());
        }
        let path = std::path::PathBuf::from(&e.path);
        let key = e.key;
        let selection = e.selection.clone();
        if matches!(action, Action::Inspect) {
            let metadata = sources::inspect(&path)?;
            self.file_objects.draft.as_mut().unwrap().metadata = Some(metadata);
            return Ok(());
        }
        let e = self.file_objects.draft.as_mut().unwrap();
        e.prepared = None;
        e.image = None;
        e.notes.clear();
        e.source_stamps.clear();
        e.review_all = matches!(action, Action::RefreshAll);
        e.acknowledged = false;
        if !matches!(action, Action::Restore | Action::RefreshAll) {
            core::local_path(&path)?;
        }
        let mut source_stamps = vec![];
        let mut notes = vec![];
        let mut prepared = match action {
            Action::Inspect => unreachable!(),
            Action::Restore => Prepared::Change {
                state: Box::new(core::restore_original(
                    d,
                    key.ok_or("Select a linked layer")?,
                )?),
                key: key.unwrap(),
            },
            Action::New => {
                let (state, id, stamps) = sources::create(d, &path, "Linked file", selection)?;
                source_stamps = stamps;
                Prepared::Change {
                    state: Box::new(state),
                    key: Key::Object(id),
                }
            }
            Action::Refresh | Action::RefreshAll => {
                let result = if matches!(action, Action::RefreshAll) {
                    sources::refresh_all(d)?
                } else {
                    sources::refresh(
                        d,
                        &[(
                            key.ok_or("Select a linked layer")?,
                            Some(path.clone()),
                            selection,
                        )],
                    )?
                };
                notes.push(format!(
                    "{} sources prepared · {} failures keep their last successful caches",
                    result.succeeded.len(),
                    result.failed.len()
                ));
                for (key, error) in &result.failed {
                    notes.push(format!("Kept {key:?}: {error}"));
                }
                let names = catalog::rows(d)?;
                for key in &result.succeeded {
                    let name = names
                        .iter()
                        .find(|r| r.key == *key)
                        .map_or("Linked source", |r| r.name.as_str());
                    let path = d
                        .scene
                        .file_objects
                        .get(*key)
                        .map_or_else(String::new, |l| l.path.display().to_string());
                    notes.push(format!("Update {name}: {path}"));
                }
                let Some(selected) = result.succeeded.first().copied() else {
                    self.file_objects.draft.as_mut().unwrap().notes = notes;
                    return Ok(());
                };
                source_stamps = result.stamps;
                Prepared::Change {
                    state: Box::new(result.state),
                    key: selected,
                }
            }
            Action::Convert => Prepared::Convert(Box::new(core::convert(
                d,
                key.ok_or("Select a layer")?,
                &path,
            )?)),
            Action::Clipboard => Prepared::Convert(Box::new(core::from_clipboard(
                d,
                packet.ok_or("Clipboard object is empty")?,
                &path,
            )?)),
        };
        if matches!(action, Action::New | Action::Clipboard) {
            let state = match &mut prepared {
                Prepared::Change { state, .. } => state.as_mut(),
                Prepared::Convert(c) => &mut c.state,
            };
            super::layer_defaults::apply(
                d,
                state,
                &self.workspaces.store.current.layer_defaults,
                self.documents.iter().map(Document::resident_bytes).sum(),
            )?;
        }
        let external = match &prepared {
            Prepared::Convert(c) => c.external.memory_bytes(),
            _ => 0,
        };
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(prepared.state().memory_bytes().saturating_mul(4))
            .saturating_add(external)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("File preview exceeds workspace memory".into());
        }
        core::validate(d, prepared.state())?;
        let page = drawer_core::layer_merge::preview(prepared.state(), d.current_frame)?;
        let image = super::layer_merge::image(ctx, &page, "linked-file-preview");
        let e = self.file_objects.draft.as_mut().unwrap();
        e.stamp = stamp(&path);
        e.source_stamps = source_stamps;
        e.notes = notes;
        e.prepared = Some(Box::new(prepared));
        e.image = Some(image);
        Ok(())
    }
    fn apply_file_object(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .file_objects
            .draft
            .as_mut()
            .ok_or("File layer editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen linked files".into());
        }
        if e.review_all && !e.acknowledged {
            return Err(
                "Review all successful updates and failures before applying the batch".into(),
            );
        }
        for source in &e.source_stamps {
            source.validate()?;
        }
        let prepared = e.prepared.as_mut().ok_or("Preview a file action first")?;
        core::validate(d, prepared.state())?;
        if let Prepared::Convert(c) = prepared.as_mut() {
            if stamp(&c.path) != e.stamp {
                return Err(
                    "Source destination changed after preview; preview and confirm it again".into(),
                );
            }
            c.save(e.overwrite)?;
        }
        let prepared = *e.prepared.take().unwrap();
        let key = prepared.key();
        let state = match prepared {
            Prepared::Change { state, .. } => *state,
            Prepared::Convert(c) => c.state,
        };
        self.documents[n].commit_state(state)?;
        if let Some(t) = catalog::thumbnail(&self.documents[n], key) {
            self.panels.targets.insert(e.id, t);
        }
        self.file_objects.draft = None;
        Ok(())
    }
    fn open_file_object_source(&mut self, ctx: &egui::Context, folder: bool) -> Result<(), String> {
        let e = self
            .file_objects
            .draft
            .as_ref()
            .ok_or("File layer editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let link = d
            .scene
            .file_objects
            .get(e.key.ok_or("Select a linked layer")?)
            .ok_or("Select a linked layer")?;
        let path = link.path.clone();
        core::local_path(&path)?;
        if folder {
            let parent = core::source_folder(&path)?;
            let folder = parent.to_string_lossy();
            let folder = folder.strip_prefix(r"\\?\").unwrap_or(&folder);
            std::process::Command::new("explorer.exe")
                .arg(folder)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        } else if link
            .path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("drawer"))
        {
            self.open_path(&path)
        } else {
            self.begin_import(ctx, &path);
            Ok(())
        }
    }
    pub(super) fn file_objects_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.file_objects.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut action, mut apply, mut cancel, mut source) = (None, false, false, None);
        egui::Window::new("Local linked file layer").open(&mut open).fixed_size(egui::vec2(600.,630.)).default_pos(egui::pos2(35.,10.)).collapsible(false).resizable(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen linked files.");return;}
            let linked=e.key.and_then(|k|d.scene.file_objects.get(k));
            ui.label("Absolute local source path / conversion destination");
            if ui.text_edit_singleline(&mut e.path).changed(){e.prepared=None;e.image=None;e.metadata=None;e.overwrite=false;e.notes.clear();e.acknowledged=false;}
            let r=ui.button("Inspect source choices");self.file_objects.inspect=Some(r.rect);if r.clicked(){action=Some(Action::Inspect);}
            ui.allocate_ui(egui::vec2(580.,140.),|ui|{egui::ScrollArea::vertical().id_salt("source-options").max_height(140.).show(ui,|ui|{
                if let Some(l)=linked{ui.small(format!("Last successful source: {}",l.path.display()));}
                if super::source_choices::controls(ui,&mut e.selection,e.metadata.as_ref()){e.prepared=None;e.image=None;e.notes.clear();e.acknowledged=false;}
                ui.small("A native document has one timeline. Choose Timeline only or a track/cell in Content, then choose its frame. TIFF sources expose their pages.");
            });});
            ui.horizontal(|ui|{let r=ui.button("Preview new link");self.file_objects.new=Some(r.rect);if r.clicked(){action=Some(Action::New);}let r=ui.add_enabled(linked.is_some(),egui::Button::new("Preview refresh / relink"));self.file_objects.refresh=Some(r.rect);if r.clicked(){action=Some(Action::Refresh);}});
            let r=ui.add_enabled(!d.scene.file_objects.entries.is_empty(),egui::Button::new("Preview refresh all linked sources"));self.file_objects.refresh_all=Some(r.rect);if r.clicked(){action=Some(Action::RefreshAll);}
            ui.horizontal(|ui|{let r=ui.button("Preview selected layer to file");self.file_objects.convert=Some(r.rect);if r.clicked(){action=Some(Action::Convert);}
if ui.button("Preview clipboard to file").clicked(){action=Some(Action::Clipboard);}});
            if matches!(e.prepared.as_deref(),Some(Prepared::Convert(_)))&&std::path::Path::new(&e.path).exists(){ui.checkbox(&mut e.overwrite,"Replace this existing source file when applying");}
            ui.allocate_ui(egui::vec2(580.,52.),|ui|{egui::ScrollArea::vertical().id_salt("source-results").max_height(52.).show(ui,|ui|{for note in &e.notes{ui.small(note);}});});
            ui.allocate_ui(egui::vec2(580.,100.),|ui|{if let Some(image)=&e.image{ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));}});
            let r=ui.add_enabled(e.review_all&&e.prepared.is_some(),egui::Checkbox::new(&mut e.acknowledged,"I reviewed the successful updates and failures in this batch"));self.file_objects.acknowledge=Some(r.rect);
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some()&&(!e.review_all||e.acknowledged),egui::Button::new("Apply reviewed file action"));self.file_objects.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
            if ui.add_enabled(linked.is_some(),egui::Button::new("Restore original embedded layer")).clicked(){action=Some(Action::Restore);}
            ui.horizontal(|ui|{if ui.add_enabled(linked.is_some(),egui::Button::new("Open source document")).clicked(){source=Some(false);}
if ui.add_enabled(linked.is_some(),egui::Button::new("Open source folder")).clicked(){source=Some(true);}});
            ui.small("Sources are local. Refresh is explicit and keeps failed caches. Successful reviewed updates form one Undo. Source changes after preview require a new preview. Conversion writes only its explicit destination.");
        });
        if open && !cancel {
            self.file_objects.draft = Some(e);
        }
        if let Some(action) = action {
            let packet = if matches!(action, Action::Clipboard) {
                match super::paste_dialog::read_clipboard() {
                    Ok(p) => Some(p),
                    Err(e) => {
                        self.status = Some(e);
                        return;
                    }
                }
            } else {
                None
            };
            if let Err(e) = self.preview_file_object(ctx, action, packet) {
                self.status = Some(e);
            }
        }
        if apply && let Err(e) = self.apply_file_object() {
            self.status = Some(e);
        }
        if let Some(folder) = source
            && let Err(e) = self.open_file_object_source(ctx, folder)
        {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_ui_link_open_source_refresh_relink_convert_undo_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
