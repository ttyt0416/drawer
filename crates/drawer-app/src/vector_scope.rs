use super::*;
use drawer_core::{
    vector_clipboard::{self as transfer, Packet},
    vector_edit::Selection,
    vector_scope::{self as core, Picked, Transform},
};
const FORMAT: &str = "Drawer.LocalVector.v1";
#[derive(Clone, Copy)]
struct Clipboard {
    write: fn(&[u8], &drawer_core::paste::ClipboardImage) -> Result<(), String>,
    read: fn() -> Result<Vec<u8>, String>,
}
pub(super) fn has_clipboard(ctx: &egui::Context) -> bool {
    if ctx
        .data(|d| d.get_temp::<Clipboard>(egui::Id::new("drawer-vector-clipboard")))
        .is_some()
    {
        return true;
    }
    clipboard_win::register_format(FORMAT).is_some_and(|f| clipboard_win::is_format_avail(f.get()))
}
fn clipboard(ctx: &egui::Context) -> Clipboard {
    ctx.data(|d|d.get_temp(egui::Id::new("drawer-vector-clipboard"))).unwrap_or(Clipboard{
    write:|bytes,image|{
        paste_dialog::write_clipboard(image,DEFAULT_DOCUMENT_BUDGET/4)?;
        let sequence=clipboard_win::seq_num();
        let _lock=clipboard_win::Clipboard::new_attempts(10).map_err(|e|e.to_string())?;
        if clipboard_win::seq_num()!=sequence{return Err("Clipboard changed; source strokes kept".into());}
        let format=clipboard_win::register_format(FORMAT).ok_or("Vector clipboard registration failed")?.get();
        clipboard_win::raw::set_without_clear(format,bytes).map_err(|e|e.to_string())
    },
    read:||{
        let format=clipboard_win::register_format(FORMAT).ok_or("Vector clipboard registration failed")?.get();
        let _lock=clipboard_win::Clipboard::new_attempts(10).map_err(|e|e.to_string())?;
        if !clipboard_win::is_format_avail(format)||clipboard_win::size(format).is_none_or(|n|n.get()>4*1024*1024){return Err("No editable Drawer vector packet; use normal image paste for the raster fallback".into());}
        clipboard_win::get(clipboard_win::formats::RawData(format)).map_err(|e|e.to_string())
    }
})
}
#[derive(Default)]
pub(super) struct Panel {
    session: Option<Box<Session>>,
    draft: Option<Box<Draft>>,
}
struct Session {
    id: u64,
    revision: u64,
    frame: u32,
    layer: Option<u64>,
    local: Selection,
    scope: Vec<Picked>,
    undo: Vec<Vec<Picked>>,
    redo: Vec<Vec<Picked>>,
}
impl Session {
    fn set(&mut self, scope: Vec<Picked>) {
        if self.scope == scope {
            return;
        }
        self.undo.push(std::mem::replace(&mut self.scope, scope));
        if self.undo.len() > 16 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    scope: Vec<Picked>,
    settings: Transform,
    all_layers: bool,
    prepared: Option<drawer_core::sizing::DocumentState>,
    result: Vec<Picked>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn vector_cleanup_scope(&mut self) -> Result<Vec<Picked>, String> {
        self.vector_scope_observe()?;
        Ok(self.vector_scope.session.as_ref().unwrap().scope.clone())
    }
    fn vector_scope_observe(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        let layer = match self.panels.targets.get(&d.id) {
            Some(drawer_core::thumbnail::Target::Vector(id)) => Some(*id),
            _ => None,
        };
        let local =
            if self.drawing.vector_document == Some(d.id) && self.drawing.vector_layer == layer {
                self.drawing.vector_selection.clone()
            } else {
                layer
                    .map(|id| Selection::current(d, id))
                    .unwrap_or_default()
            };
        if self
            .vector_scope
            .session
            .as_ref()
            .is_none_or(|s| s.id != d.id)
        {
            let scope = layer
                .filter(|_| !local.path_ids().is_empty())
                .map(|layer| {
                    vec![Picked {
                        layer,
                        selection: local.clone(),
                    }]
                })
                .unwrap_or_default();
            self.vector_scope.session = Some(Box::new(Session {
                id: d.id,
                revision: d.revision,
                frame: d.current_frame,
                layer,
                local: local.clone(),
                scope,
                undo: vec![],
                redo: vec![],
            }));
        }
        let s = self.vector_scope.session.as_mut().unwrap();
        if (s.revision, s.frame) != (d.revision, d.current_frame) {
            s.revision = d.revision;
            s.frame = d.current_frame;
            s.undo.clear();
            s.redo.clear();
            s.scope
                .retain(|p| core::validate(d, std::slice::from_ref(p)).is_ok());
        }
        if !self.drawing.pending() {
            if s.layer != layer {
                s.layer = layer;
                s.local = local;
            } else if s.local != local {
                let mut next = s.scope.clone();
                next.retain(|p| Some(p.layer) != layer);
                if let Some(layer) = layer.filter(|_| !local.path_ids().is_empty()) {
                    next.push(Picked {
                        layer,
                        selection: local.clone(),
                    });
                }
                s.set(next);
                s.local = local;
            }
        }
        Ok(())
    }
    fn vector_scope_set(&mut self, scope: Vec<Picked>, record: bool) -> Result<(), String> {
        self.vector_scope_observe()?;
        let s = self.vector_scope.session.as_mut().unwrap();
        if record {
            s.set(scope);
        } else {
            s.scope = scope;
            s.undo.clear();
            s.redo.clear();
        }
        let local = s
            .layer
            .and_then(|id| s.scope.iter().find(|p| p.layer == id))
            .map(|p| p.selection.clone())
            .unwrap_or_default();
        s.local = local.clone();
        self.drawing.vector_selection = local;
        Ok(())
    }
    pub(super) fn vector_scope_history(&mut self, redo: bool) -> Result<(), String> {
        self.finish_stroke();
        self.vector_scope_observe()?;
        let s = self.vector_scope.session.as_mut().unwrap();
        let next = if redo {
            s.redo.pop().ok_or("No vector selection to redo")?
        } else {
            s.undo.pop().ok_or("No vector selection to undo")?
        };
        if redo {
            s.undo.push(std::mem::replace(&mut s.scope, next));
        } else {
            s.redo.push(std::mem::replace(&mut s.scope, next));
        }
        let local = s
            .layer
            .and_then(|id| s.scope.iter().find(|p| p.layer == id))
            .map(|p| p.selection.clone())
            .unwrap_or_default();
        s.local = local.clone();
        self.drawing.vector_selection = local;
        Ok(())
    }
    pub(super) fn begin_vector_scope(&mut self) -> Result<(), String> {
        self.finish_stroke();
        self.vector_scope_observe()?;
        let d = &self.documents[self.active.unwrap()];
        let s = self.vector_scope.session.as_ref().unwrap();
        self.vector_scope.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            scope: s.scope.clone(),
            settings: Default::default(),
            all_layers: false,
            prepared: None,
            result: vec![],
            images: None,
        }));
        Ok(())
    }
    fn select_vector_area(&mut self, contained: bool, all: bool) -> Result<(), String> {
        self.finish_stroke();
        self.vector_scope_observe()?;
        let d = &self.documents[self.active.unwrap()];
        let layers = if all {
            d.scene.lines.layers.iter().map(|l| l.id).collect()
        } else {
            vec![
                self.vector_scope
                    .session
                    .as_ref()
                    .unwrap()
                    .layer
                    .ok_or("Choose a vector layer")?,
            ]
        };
        let selected = core::area(d, &layers, contained)?;
        self.vector_scope_set(selected, true)?;
        self.begin_vector_scope()?;
        self.vector_scope.draft.as_mut().unwrap().all_layers = all;
        Ok(())
    }
    pub(super) fn vector_area_command(&mut self, contained: bool) -> Result<(), String> {
        let all = self
            .vector_scope
            .draft
            .as_ref()
            .is_some_and(|e| e.all_layers);
        self.select_vector_area(contained, all)
    }
    fn prepare_vector_transform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.finish_stroke();
        self.vector_scope_observe()?;
        let d = &self.documents[self.active.unwrap()];
        let e = self
            .vector_scope
            .draft
            .as_mut()
            .ok_or("Open vector scope tools")?;
        e.prepared = None;
        e.images = None;
        if (e.id, e.revision, e.frame) != (d.id, d.revision, d.current_frame)
            || e.scope != self.vector_scope.session.as_ref().unwrap().scope
        {
            return Err("Page/frame/selection changed; load the current selection".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Vector preview workspace memory limit".into());
        }
        let (state, result) = core::transform(d, &e.scope, e.settings)?;
        let after = drawing::bucket::preview(d, state.clone())?;
        e.images = Some([
            layer_merge::image(ctx, d, "vector-scope-before"),
            layer_merge::image(ctx, &after, "vector-scope-after"),
        ]);
        e.result = result;
        e.prepared = Some(state);
        Ok(())
    }
    fn apply_vector_transform(&mut self) -> Result<(), String> {
        self.vector_scope_observe()?;
        let n = self.active.unwrap();
        let d = &self.documents[n];
        let e = self
            .vector_scope
            .draft
            .as_mut()
            .ok_or("Open vector scope tools")?;
        if (e.id, e.revision, e.frame) != (d.id, d.revision, d.current_frame)
            || e.scope != self.vector_scope.session.as_ref().unwrap().scope
        {
            return Err("Page/frame/selection changed; load the current selection".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Prepare a vector transform preview")?;
        let result = e.result.clone();
        self.documents[n].commit_state(state)?;
        self.vector_scope_set(result, false)?;
        self.begin_vector_scope()
    }
    pub(super) fn copy_vector_scope(
        &mut self,
        ctx: &egui::Context,
        spans: bool,
        cut: bool,
    ) -> Result<(), String> {
        self.finish_stroke();
        self.vector_scope_observe()?;
        let n = self.active.unwrap();
        let d = &self.documents[n];
        let scope = self.vector_scope.session.as_ref().unwrap().scope.clone();
        if self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>()
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Vector clipboard workspace memory limit".into());
        }
        let packet = transfer::copy(d, &scope, spans)?;
        let bytes = packet.encode()?;
        let raster = packet.raster()?;
        let delete = if cut {
            Some(transfer::cut(d, &scope, spans)?)
        } else {
            None
        };
        (clipboard(ctx).write)(&bytes, &raster)?;
        if let Some(state) = delete {
            self.documents[n].commit_state(state)?;
            self.vector_scope_set(vec![], false)?;
        }
        self.status=Some(if spans{"Vector spans copied as editable outlines; pressure is interpolated at new endpoints. Raster fallback uses their rendered bounds."}else{"Whole strokes containing any selected control copied; geometry, handles, pressure and style retained. Raster fallback uses the copied strokes' rendered bounds."}.into());
        Ok(())
    }
    pub(super) fn paste_vector_scope(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.finish_stroke();
        self.vector_scope_observe()?;
        let n = self.active.unwrap();
        let d = &self.documents[n];
        let layer = self
            .vector_scope
            .session
            .as_ref()
            .unwrap()
            .layer
            .ok_or("Choose a vector layer")?;
        if self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>()
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Vector paste workspace memory limit".into());
        }
        let packet = Packet::decode(&(clipboard(ctx).read)()?)?;
        let (state, selected) = transfer::paste(d, layer, &packet, [0.; 2])?;
        self.documents[n].commit_state(state)?;
        self.vector_scope_set(vec![selected], false)?;
        Ok(())
    }
    pub(super) fn vector_scope_window(&mut self, ctx: &egui::Context) {
        if self.active.is_none() {
            self.vector_scope.session = None;
            self.vector_scope.draft = None;
            return;
        }
        let _ = self.vector_scope_observe();
        let Some(mut e) = self.vector_scope.draft.take() else {
            return;
        };
        let d = &self.documents[self.active.unwrap()];
        if d.id != e.id {
            return;
        }
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame)
            && self
                .vector_scope
                .session
                .as_ref()
                .is_some_and(|s| s.scope == e.scope);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut reload) = (false, false, false);
        let mut area = None;
        let mut clip = None;
        let mut history = None;
        let mut paste = false;
        let mut editor = false;
        let mut clear = false;
        egui::Window::new("Vector selection, pivot and transfer").open(&mut open).collapsible(false).default_width(470.).vscroll(true).show(ctx,|ui|{
            ui.label(format!("{} vector layers · {} selected paths",e.scope.len(),e.scope.iter().map(|p|p.selection.path_ids().len()).sum::<usize>()));
            reload=ui.button("Load current points / strokes").clicked();
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page, frame or selection changed. Reload before transforming.");}
            ui.horizontal(|ui|{if ui.button("Undo vector selection").clicked(){history=Some(false);}
                if ui.button("Redo vector selection").clicked(){history=Some(true);}});
            ui.checkbox(&mut e.all_layers,"Area selection across all editable vector layers");
            ui.horizontal(|ui|{if ui.button("Select overlapping centerlines").clicked(){area=Some(false);}
                if ui.button("Select contained centerlines").clicked(){area=Some(true);}});
            ui.small("Centerlines determine overlap/containment; boundary contact is inside. Stroke width is ignored. Locked/hidden layers are skipped. Selection history changes no artwork or drawing Undo.");
            let mut changed=false;
            ui.horizontal(|ui|{ui.label("Pivot in document pixels");for (v,label) in e.settings.pivot.iter_mut().zip(["X: ","Y: "]){changed|=ui.add(egui::DragValue::new(v).speed(0.1).prefix(label).range(-1e9..=1e9)).changed();}});
            ui.horizontal(|ui|{
                if ui.button("First selected anchor").clicked() && let Some(entry)=e.scope.first() && let Some(layer)=d.scene.lines.layers.iter().find(|l|l.id==entry.layer) && let Some(path)=layer.paths.iter().find(|p|entry.selection.path_ids().contains(&p.id)) {
                    let n=entry.selection.controls.iter().find(|c|c.path==path.id).map_or(0,|c|c.point as usize);
                    if let Some(p)=path.control_point(n){e.settings.pivot=d.scene.transform.map(p);changed=true;}
                }
                if ui.button("Last canvas layer-pick point").clicked()&&let Some((id,p))=self.drawing.pick_point&&id==d.id{e.settings.pivot=p;changed=true;}
            });
            ui.horizontal(|ui|{for (v,label) in e.settings.translation.iter_mut().zip(["Move X: ","Y: "]){changed|=ui.add(egui::DragValue::new(v).speed(0.1).prefix(label).range(-1e9..=1e9)).changed();}});
            ui.horizontal(|ui|{for (v,label) in e.settings.scale.iter_mut().zip(["Scale X: ","Y: "]){changed|=ui.add(egui::DragValue::new(v).speed(0.01).prefix(label).range(-1e4..=1e4)).changed();}changed|=ui.add(egui::DragValue::new(&mut e.settings.degrees).speed(0.1).prefix("Rotate: ").range(-36000. ..=36000.)).changed();});
            changed|=ui.checkbox(&mut e.settings.preserve_width,"Keep line width (otherwise geometric mean scale)").changed();
            changed|=ui.checkbox(&mut e.settings.individual,"Individual selected path / anchor-set centers").changed();
            if ui.button("Use selected set center").clicked() {
                let points=e.scope.iter().flat_map(|entry|d.scene.lines.layers.iter().filter(move|l|l.id==entry.layer).flat_map(move|l|l.paths.iter().filter(move|p|entry.selection.path_ids().contains(&p.id)).flat_map(|p|p.points.iter().copied())));
                if let Ok(p)=drawer_core::transform_policy::center(points.map(|p|d.scene.transform.map(p))){e.settings.pivot=p;changed=true;}
            }
            changed|=ui.checkbox(&mut e.settings.duplicate,"Duplicate selected paths, then transform the selected scope in the copies").changed();
            ui.small("Selected strokes transform in full; point selections transform only those anchors and their node handles. Line width follows the selected policy. Duplicate creates fresh IDs and preserves every original path.");
            if changed{e.prepared=None;e.images=None;}
            preview=ui.add_enabled(fresh&&!e.scope.is_empty(),egui::Button::new("Prepare pivot transform preview")).clicked();
            if let Some(images)=&e.images{ui.horizontal(|ui|{for (label,image) in ["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}
            apply=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply vector transform")).clicked();
            ui.separator();ui.horizontal_wrapped(|ui|{for (label,spans,cut) in [("Copy whole selected strokes",false,false),("Cut whole selected strokes",false,true),("Copy selected spans",true,false),("Cut selected spans",true,true)]{if ui.button(label).clicked(){clip=Some((spans,cut));}}if ui.button("Paste editable vectors").clicked(){paste=true;}});
            ui.small("Standard vector copy includes whole strokes with any selected point. Span copy/cut requires adjacent selected anchors; it creates editable outline endpoints with interpolated pressure. Filled shapes require whole-stroke transfer. Clipboard write must succeed before cut deletes anything. Raster fallback includes copied content bounds. Editable paste keeps a matching page scale/rotation frame; other frames can use the raster fallback or SVG.");
            if ui.button("Open point / join / corner / smooth tools…").clicked(){editor=true;}
            clear=ui.button("Clear vector scope selection").clicked();
        });
        if open {
            self.vector_scope.draft = Some(e);
        }
        let result = if clear {
            self.vector_scope_set(vec![], true)
                .and_then(|_| self.begin_vector_scope())
        } else if editor {
            self.vector_scope.draft = None;
            self.begin_vector_edit(None)
        } else if paste {
            self.paste_vector_scope(ctx)
        } else if reload {
            self.begin_vector_scope()
        } else if let Some(redo) = history {
            self.vector_scope_history(redo)
                .and_then(|_| self.begin_vector_scope())
        } else if let Some(contained) = area {
            let all = self
                .vector_scope
                .draft
                .as_ref()
                .is_some_and(|e| e.all_layers);
            self.select_vector_area(contained, all)
        } else if preview {
            self.prepare_vector_transform(ctx)
        } else if apply {
            self.apply_vector_transform()
        } else if let Some((spans, cut)) = clip {
            self.copy_vector_scope(ctx, spans, cut)
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.status = Some(error);
        }
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_scope_verify::verify(root)?;
    thread_local! {static BYTES:std::cell::RefCell<Vec<u8>>=const{std::cell::RefCell::new(Vec::new())};static FAIL:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};static SIZE:std::cell::Cell<(u32,u32)>=const{std::cell::Cell::new((0,0))};}
    fn write(bytes: &[u8], image: &drawer_core::paste::ClipboardImage) -> Result<(), String> {
        if FAIL.get() {
            return Err("Simulated clipboard write failure".into());
        }
        BYTES.with(|v| *v.borrow_mut() = bytes.to_vec());
        SIZE.set((image.image.width, image.image.height));
        Ok(())
    }
    fn read() -> Result<Vec<u8>, String> {
        if FAIL.get() {
            return Err("Simulated clipboard read failure".into());
        }
        Ok(BYTES.with(|v| v.borrow().clone()))
    }
    let ctx = egui::Context::default();
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new("drawer-vector-clipboard"),
            Clipboard { write, read },
        )
    });
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::vector_scope_verify::fixture()?.snapshot())?;
    let layer = app.documents[0].scene.lines.layers[0].id;
    let path = app.documents[0].scene.lines.layers[0].paths[0].id;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Vector(layer),
    );
    app.begin_vector_edit(Some(drawer_core::vector_edit::Tool::Point))?;
    let selection = Selection {
        controls: vec![
            drawer_core::vector_selection::Control { path, point: 1 },
            drawer_core::vector_selection::Control { path, point: 2 },
        ],
        paths: vec![],
    };
    app.drawing.vector_selection = selection.clone();
    app.vector_scope_observe()?;
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    app.begin_vector_edit(None)?;
    if app.drawing.vector_selection != selection {
        return Err("Point editor discarded session scope".into());
    }
    app.vector_edit = Default::default();
    app.execute_command(&ctx, "vector-scope-tools");
    app.vector_scope.draft.as_mut().unwrap().settings = Transform {
        pivot: [16., 8.],
        translation: [4., 2.],
        degrees: 90.,
        duplicate: true,
        ..Default::default()
    };
    app.prepare_vector_transform(&ctx)?;
    let planned = app
        .vector_scope
        .draft
        .as_ref()
        .unwrap()
        .prepared
        .clone()
        .unwrap();
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history {
        return Err("Pivot preview changed source".into());
    }
    crate::input_replay::frame(&ctx, &mut app, 0.1, vec![]);
    app.apply_vector_transform()?;
    if app.documents[0].snapshot() != planned
        || app.documents[0].history_counts().0 != history.0 + 1
        || app.documents[0].scene.lines.layers[0].paths[..2] != before.scene.lines.layers[0].paths
    {
        return Err("Pivot duplicate apply/source/one Undo".into());
    }
    let file = root.join("app-vector-scope.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != planned
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err("Pivot duplicate native/Undo".into());
    }
    app.vector_scope.draft = None;
    app.drawing.vector_selection = selection.clone();
    app.vector_scope_observe()?;
    app.execute_command(&ctx, "copy-selection");
    let bytes = BYTES.with(|v| v.borrow().clone());
    let packet = Packet::decode(&bytes)?;
    if packet.paths != vec![before.scene.lines.layers[0].paths[0].clone()]
        || SIZE.get().0 < 40
        || app.documents[0].snapshot() != before
    {
        return Err(format!(
            "Default copy did not keep whole stroke bounds: {:?}",
            app.status
        ));
    }
    app.execute_command(&ctx, "paste-selection");
    if app.documents[0].scene.lines.layers[0].paths.len() != 3
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err(format!("Default vector paste/Undo: {:?}", app.status));
    }
    app.drawing.vector_selection = selection.clone();
    app.vector_scope_observe()?;
    FAIL.set(true);
    let count = app.documents[0].history_counts();
    app.execute_command(&ctx, "vector-cut-strokes");
    if app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != count
        || BYTES.with(|v| *v.borrow() != bytes)
    {
        return Err("Failed clipboard cut deleted data".into());
    }
    app.execute_command(&ctx, "vector-paste");
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != count {
        return Err("Failed clipboard paste changed source".into());
    }
    FAIL.set(false);
    app.execute_command(&ctx, "vector-copy-spans");
    let part = Packet::decode(&BYTES.with(|v| v.borrow().clone()))?;
    if part.paths[0].points != [[16., 8.], [32., 12.]] || app.documents[0].snapshot() != before {
        return Err("App span copy policy/source".into());
    }
    app.execute_command(&ctx, "vector-cut-spans");
    if app.documents[0].scene.lines.layers[0].paths.len() != 3
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err(format!("Span cut one Undo: {:?}", app.status));
    }
    app.drawing.vector_selection = selection;
    app.vector_scope_observe()?;
    app.begin_vector_scope()?;
    app.prepare_vector_transform(&ctx)?;
    crate::input_replay::frame(
        &ctx,
        &mut app,
        0.2,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.vector_scope.draft.is_some() || app.documents[0].snapshot() != before {
        return Err("Pivot Escape committed preview".into());
    }
    crate::input_replay::frame(
        &ctx,
        &mut app,
        0.3,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    app.begin_vector_scope()?;
    app.prepare_vector_transform(&ctx)?;
    app.drawing.vector_selection = Selection {
        controls: vec![],
        paths: vec![path],
    };
    if app.apply_vector_transform().is_ok() || app.documents[0].snapshot() != before {
        return Err("Changed selection accepted an old pivot preview".into());
    }
    app.vector_scope.draft = None;
    let old = app.vector_scope.session.as_ref().unwrap().scope.clone();
    let count = app.documents[0].history_counts();
    app.vector_scope_set(vec![], true)?;
    app.vector_scope_history(false)?;
    if app.vector_scope.session.as_ref().unwrap().scope != old
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != count
    {
        return Err("Selection history changed artwork/Undo".into());
    }
    app.vector_scope_history(true)?;
    if !app.vector_scope.session.as_ref().unwrap().scope.is_empty() {
        return Err("Selection redo did not restore empty set".into());
    }
    app.documents[0].scene.selection = Some(drawer_core::scene::Rect {
        min: [8., 6.],
        max: [36., 16.],
    });
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    app.select_vector_area(false, true)?;
    if app.vector_scope.session.as_ref().unwrap().scope.is_empty()
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != count
    {
        return Err("Area command changed document/history".into());
    }
    app.begin_vector_scope()?;
    app.prepare_vector_transform(&ctx)?;
    app.documents[0].revision += 1;
    if app.apply_vector_transform().is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale page pivot preview accepted".into());
    }
    app.vector_scope.draft = None;
    BYTES.with(|v| v.borrow_mut().clear());
    Ok(())
}

pub(super) fn read_timeline_packet(ctx: &egui::Context) -> Result<Packet, String> {
    Packet::decode(&(clipboard(ctx).read)()?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn vector_scope_ui_clipboard_failure_pivot_cancel_selection_history() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
