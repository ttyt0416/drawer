use super::*;
use drawer_core::{
    line_tools::Target,
    pointer_input::{Decision, Phase, Source},
    vector_erase::{self as core, Mode, Settings},
};
pub(super) struct Gesture {
    pub document: u64,
    pub view: u64,
    revision: u64,
    frame: u32,
    source: Source,
    projection: Projection,
    root: egui::Rect,
    target: Option<Target>,
    settings: Settings,
    last: Point,
    samples: usize,
    preview: Document,
}
impl Drawing {
    fn erase_target(&self) -> Option<Target> {
        match self.auto_target {
            Some(drawer_core::shapes::Target::Raster(n)) => Some(Target::Raster(n)),
            Some(drawer_core::shapes::Target::Vector(Some(id))) => Some(Target::Vector(id)),
            _ => None,
        }
    }
    pub(crate) fn vector_erase_preview(&self, id: u64) -> Option<&Document> {
        self.vector_eraser
            .as_ref()
            .filter(|s| s.document == id)
            .map(|s| &s.preview)
    }
    pub(super) fn advance_vector_erase(&mut self, ctx: &egui::Context, docs: &mut [Document]) {
        if self.vector_eraser.as_ref().is_some_and(|s| {
            self.mode != crate::precision::Mode::VectorErase
                || s.settings != self.erase_settings
                || s.target != self.erase_target()
                || s.root != ctx.content_rect()
                || (s.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8
                || docs.iter().find(|d| d.id == s.document).is_none_or(|d| {
                    (d.revision, d.current_frame, d.view)
                        != (s.revision, s.frame, s.projection.view)
                })
        }) {
            self.vector_eraser = None;
            self.ownership.clear();
        }
        for n in 0..self.events.len() {
            let Some(id) = self.vector_eraser.as_ref().map(|s| s.document) else {
                break;
            };
            if let Some(d) = docs.iter_mut().find(|d| d.id == id) {
                self.receive_vector_erase(n, d);
            }
        }
    }
    fn receive_vector_erase(&mut self, n: usize, d: &mut Document) {
        if self.events[n].1 {
            return;
        }
        let drawer_core::touch::Effect::Pointer(e) = self.events[n].0 else {
            return;
        };
        let Some(mut s) = self.vector_eraser.take() else {
            return;
        };
        if s.document != d.id || s.source != e.source {
            self.vector_eraser = Some(s);
            return;
        }
        self.events[n].1 = true;
        if e.phase == Phase::Cancel {
            self.ownership.clear();
            return;
        }
        let result = (|| {
            if matches!(e.phase, Phase::Move | Phase::Up) {
                let point = s
                    .projection
                    .point(egui::pos2(e.position[0] as f32, e.position[1] as f32));
                let distance = (point[0] - s.last[0]).hypot(point[1] - s.last[1]);
                let count = (distance / (s.settings.radius / 2.).max(0.05)).ceil() as usize;
                if s.samples.saturating_add(count) > 4096 {
                    return Err("Eraser gesture sample limit; drag cancelled".into());
                }
                if s.samples
                    .saturating_add(count)
                    .saturating_mul(d.memory_bytes())
                    > 512 * 1024 * 1024
                {
                    return Err("Eraser gesture work limit; drag cancelled".into());
                }
                for part in 1..=count {
                    let t = part as f64 / count as f64;
                    let p = [
                        s.last[0] + (point[0] - s.last[0]) * t,
                        s.last[1] + (point[1] - s.last[1]) * t,
                    ];
                    let state = core::prepare(&s.preview, s.target, p, s.settings)?;
                    s.preview = bucket::preview(d, state)?;
                }
                s.samples += count;
                s.last = point;
                if e.phase == Phase::Up {
                    d.commit_state(s.preview.snapshot())?;
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        match result {
            Ok(false) => self.vector_eraser = Some(s),
            Ok(true) => self.ownership.clear(),
            Err(error) => {
                self.error = Some(error);
                self.ownership.clear();
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn vector_erase_canvas(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        d: &mut Document,
        view: u64,
        projection: Projection,
        enabled: bool,
    ) {
        if self
            .vector_eraser
            .as_ref()
            .is_some_and(|s| s.document == d.id && (s.view != view || s.projection != projection))
        {
            self.vector_eraser = None;
            self.ownership.clear();
        }
        for n in 0..self.events.len() {
            if self.vector_eraser.is_some() {
                self.receive_vector_erase(n, d);
                continue;
            }
            if self.events[n].1 {
                continue;
            }
            let drawer_core::touch::Effect::Pointer(e) = self.events[n].0 else {
                continue;
            };
            let pos = egui::pos2(e.position[0] as f32, e.position[1] as f32);
            if !enabled
                || e.phase != Phase::Down
                || !response.rect.contains(pos)
                || !ui.clip_rect().contains(pos)
                || ui.ctx().layer_id_at(pos) != Some(response.layer_id)
            {
                continue;
            }
            if self.ownership.receive(e, self.settings) != Decision::Begin {
                continue;
            }
            self.events[n].1 = true;
            let point = projection.point(pos);
            let target = self.erase_target();
            let result = core::prepare(d, target, point, self.erase_settings)
                .and_then(|state| bucket::preview(d, state));
            match result {
                Ok(preview) => {
                    self.vector_eraser = Some(Box::new(Gesture {
                        document: d.id,
                        view,
                        revision: d.revision,
                        frame: d.current_frame,
                        source: e.source,
                        projection,
                        root: ui.ctx().content_rect(),
                        target,
                        settings: self.erase_settings,
                        last: point,
                        samples: 1,
                        preview,
                    }))
                }
                Err(error) => {
                    self.error = Some(error);
                    self.ownership.clear();
                }
            }
        }
        if response.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Crosshair);
        }
    }
    pub(crate) fn vector_erase_overlay(
        &self,
        ui: &egui::Ui,
        d: &Document,
        original_id: u64,
        view: u64,
        rect: egui::Rect,
    ) {
        if self.mode != crate::precision::Mode::VectorErase
            || self.vector_document != Some(original_id)
            || self.active_view().is_some_and(|v| v != view)
        {
            return;
        }
        let position = self
            .events
            .iter()
            .rev()
            .find_map(|(e, _)| match e {
                drawer_core::touch::Effect::Pointer(e) => {
                    Some(egui::pos2(e.position[0] as f32, e.position[1] as f32))
                }
                _ => None,
            })
            .or_else(|| ui.input(|i| i.pointer.hover_pos()));
        if let Some(pos) = position.filter(|p| rect.contains(*p)) {
            ui.painter_at(rect).circle_stroke(
                pos,
                (self.erase_settings.radius * d.view.zoom / ui.ctx().pixels_per_point() as f64)
                    as f32,
                egui::Stroke::new(1., egui::Color32::YELLOW),
            );
        }
    }
}
impl Drawer {
    pub(crate) fn begin_vector_eraser(&mut self) {
        self.finish_stroke();
        self.restore_locked_tool();
        self.drawing.mode = crate::precision::Mode::VectorErase;
        self.drawing.connect_line = false;
        self.vector_eraser_open = true;
    }
    pub(crate) fn delete_vector_strokes(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        let layer = match self.panels.targets.get(&d.id) {
            Some(drawer_core::thumbnail::Target::Vector(id)) => *id,
            _ => return Err("Choose a vector layer and select strokes".into()),
        };
        let selection = if self.drawing.vector_document == Some(d.id)
            && self.drawing.vector_layer == Some(layer)
        {
            self.drawing.vector_selection.clone()
        } else {
            drawer_core::vector_edit::Selection::current(d, layer)
        };
        let state = core::delete(d, layer, &selection)?;
        self.documents[n].commit_state(state)?;
        self.drawing.vector_selection = Default::default();
        Ok(())
    }
    pub(crate) fn vector_eraser_window(&mut self, ctx: &egui::Context) {
        if !self.vector_eraser_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Vector eraser").open(&mut open).default_pos(egui::pos2(670.,55.)).default_width(300.).collapsible(false).show(ctx,|ui|{
            let s=&mut self.workspaces.store.current.vector_erase;
            egui::ComboBox::from_id_salt("vector-erase-mode").selected_text(s.mode.label()).show_ui(ui,|ui|{for mode in Mode::ALL {ui.selectable_value(&mut s.mode,mode,mode.label());}});
            ui.add(egui::DragValue::new(&mut s.radius).speed(0.1).range(0.1..=1024.).prefix("Radius in document pixels: "));
            ui.checkbox(&mut s.refer_all,"Refer all layers");ui.checkbox(&mut s.erase_all,"Erase on all layers");
            ui.small("References read visible vector paths, including locked references. Erase on all layers edits all visible unlocked raster/vector layers. Protected and linked sources are retained. Other object types are retained.");
            ui.small("Do not erase preserves vectors; raster contact erasing remains active. Whole line also removes filled vector objects. Other modes require outline paths. Clear a raster selection before vector erasing.");
            ui.small("Drag to preview; release commits one Undo. Escape or focus/page/view/tool changes cancel the entire drag. At an exact intersection, the forward interval is selected. Settings are stored locally.");
            if ui.button("Select whole vector strokes").clicked() {self.execute_command(ui.ctx(),"vector-stroke-tool");}
            if ui.button("Delete selected strokes").clicked() {self.execute_command(ui.ctx(),"vector-delete-strokes");}
        });
        self.vector_eraser_open = open;
    }
}
pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_erase_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::vector_erase_verify::fixture()?.snapshot())?;
    app.documents[0].view.zoom = 3.;
    let layer = app.documents[0].scene.lines.layers[0].id;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Vector(layer),
    );
    let mut time = 0.;
    let step = |app: &mut Drawer, t: &mut f64, events| {
        *t += 0.1;
        crate::input_replay::frame(&ctx, app, *t, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "vector-stroke-tool");
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    for phase in [Phase::Down, Phase::Up] {
        let e = crate::precision::pointer(&ctx, &app, [8., 16.], phase);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
    }
    if app.documents[0].scene.lines.selection.paths.len() != 1
        || app.documents[0].scene.lines.layers != before.scene.lines.layers
    {
        return Err(format!(
            "Whole stroke selection changed ink/missing: {:?}",
            app.status
        ));
    }
    let selected = app.documents[0].snapshot();
    app.execute_command(&ctx, "vector-delete-strokes");
    if !app.documents[0].scene.lines.layers[0].paths.is_empty()
        || app.documents[0].scene.lines.layers[1..] != before.scene.lines.layers[1..]
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != selected
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts().0 != count.0
    {
        return Err("Whole stroke selection/delete/Undo".into());
    }
    app.workspaces.store.current.vector_erase = Settings {
        mode: Mode::Intersection,
        refer_all: true,
        erase_all: false,
        radius: 0.5,
    };
    app.execute_command(&ctx, "vector-eraser-tool");
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    let down = crate::precision::pointer(&ctx, &app, [16., 16.], Phase::Down);
    app.drawing.virtual_events(vec![down], false);
    step(&mut app, &mut time, vec![]);
    let preview = app
        .drawing
        .vector_erase_preview(app.documents[0].id)
        .ok_or_else(|| format!("Eraser preview missing: {:?}", app.status))?;
    if preview.scene.lines.layers[0].paths.len() != 2
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != count
    {
        return Err("Intersection drag preview changed source or wrong parts".into());
    }
    let mut up = down;
    up.phase = Phase::Up;
    app.drawing.virtual_events(vec![up], false);
    step(&mut app, &mut time, vec![]);
    if app.drawing.pending()
        || app.documents[0].scene.lines.layers[0].paths.len() != 2
        || app.documents[0].history_counts().0 != count.0 + 1
    {
        return Err(format!(
            "Intersection mouse release failed: {:?}",
            app.status
        ));
    }
    let file = root.join("vector-eraser-input.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err("Eraser input save/reopen/Undo".into());
    }
    for cancel in 0..5 {
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
        app.drawing.virtual_events(vec![down], false);
        step(&mut app, &mut time, vec![]);
        let mut moved = crate::precision::pointer(&ctx, &app, [24., 16.], Phase::Move);
        app.drawing.virtual_events(vec![moved], false);
        step(&mut app, &mut time, vec![]);
        let events = match cancel {
            0 => vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            1 => vec![egui::Event::WindowFocused(false)],
            2 => {
                moved.phase = Phase::Cancel;
                app.drawing.virtual_events(vec![moved], false);
                vec![]
            }
            3 => {
                app.workspaces.store.current.vector_erase.mode = Mode::Touched;
                vec![]
            }
            _ => {
                app.documents[0].view.angle = 30.;
                vec![]
            }
        };
        step(&mut app, &mut time, events);
        if app.drawing.pending()
            || app.documents[0].snapshot() != before
            || app.documents[0].history_counts() != count
        {
            return Err(format!("Eraser cancel {cancel} failed: {:?}", app.status));
        }
        app.documents[0].view.angle = 0.;
        app.workspaces.store.current.vector_erase.mode = Mode::Intersection;
        step(
            &mut app,
            &mut time,
            vec![
                egui::Event::WindowFocused(true),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    app.workspaces.store.current.vector_erase.mode = Mode::Touched;
    let before = app.documents[0].snapshot();
    let count = app.documents[0].history_counts();
    for (phase, p) in [
        (Phase::Down, [8., 16.]),
        (Phase::Move, [10., 16.]),
        (Phase::Up, [14., 16.]),
    ] {
        let e = crate::precision::pointer(&ctx, &app, p, phase);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
    }
    if app.documents[0].snapshot() == before
        || app.documents[0].history_counts().0 != count.0 + 1
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
    {
        return Err(format!("Touched sweep one Undo: {:?}", app.status));
    }
    app.workspaces.store.current.vector_erase.mode = Mode::None;
    for phase in [Phase::Down, Phase::Up] {
        let e = crate::precision::pointer(&ctx, &app, [16., 16.], phase);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time, vec![]);
    }
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != count {
        return Err("Do not erase created a document edit".into());
    }
    let file = root.join("vector-eraser-workspace.json");
    app.workspaces
        .store
        .current
        .export(&file)
        .map_err(|e| e.to_string())?;
    let reopened = drawer_core::workspaces::Snapshot::read(&file).map_err(|e| e.to_string())?;
    if !reopened.valid() || reopened.vector_erase != app.workspaces.store.current.vector_erase {
        return Err("Eraser local settings reopen".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_eraser_selection_delete_drag_preview_cancel_save_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
