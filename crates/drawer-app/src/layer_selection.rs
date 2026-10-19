use super::*;
use drawer_core::{
    layer_selection::{self as alpha, Source},
    selection_edit::Combine,
    vector_selection::{self as vector, Kind},
};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    alpha_button: Option<egui::Rect>,
    vector_button: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    source: Source,
    combine: Combine,
    layer: Option<u64>,
    kind: Kind,
    width: bool,
}
impl Drawer {
    pub(super) fn begin_layer_selection(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let source = match self.panels.targets.get(&d.id) {
            Some(drawer_core::thumbnail::Target::Vector(id)) => Source::Vector(*id),
            Some(drawer_core::thumbnail::Target::Raster(n)) => Source::Raster(*n),
            _ => Source::Raster(0),
        };
        self.layer_selection.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            source,
            combine: Combine::Replace,
            layer: d
                .scene
                .lines
                .selection
                .layer
                .or_else(|| d.scene.lines.layers.first().map(|l| l.id)),
            kind: d.scene.lines.selection.kind,
            width: d.scene.lines.selection.include_width,
        }));
        Ok(())
    }
    fn apply_layer_selection(&mut self, vector: bool) -> Result<(), String> {
        let e = self
            .layer_selection
            .draft
            .as_mut()
            .ok_or("Layer selection editor is closed")?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen layer selection".into());
        }
        let scene = if vector {
            vector::prepare(
                d,
                e.layer.ok_or("Choose a vector layer")?,
                e.kind,
                e.width,
                e.combine,
            )?
        } else {
            alpha::prepare(d, e.source, e.combine)?
        };
        d.edit_scene(scene)?;
        e.revision = d.revision;
        Ok(())
    }
    pub(super) fn alpha_selection_command(&mut self, op: Combine) -> Result<(), String> {
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        let source = match self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
        {
            drawer_core::thumbnail::Target::Raster(n) => Source::Raster(n),
            drawer_core::thumbnail::Target::Vector(id) => Source::Vector(id),
            _ => {
                return Err(
                    "Choose a raster/vector layer or use the layer selection dialog".into(),
                );
            }
        };
        let scene = alpha::prepare(d, source, op)?;
        d.edit_scene(scene)?;
        Ok(())
    }
    pub(super) fn clear_vector_selection(&mut self) -> Result<(), String> {
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        let scene = vector::clear(d);
        d.edit_scene(scene)?;
        Ok(())
    }
    pub(super) fn toggle_selection_outline(&mut self) {
        self.workspaces.store.current.selection_outline =
            !self.workspaces.store.current.selection_outline;
        self.canvas_view.selection_outline = self.workspaces.store.current.selection_outline;
    }
    pub(super) fn layer_selection_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.layer_selection.draft.as_mut() else {
            return;
        };
        let Some(n) = self.active else { return };
        let d = &self.documents[n];
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut action = None;
        let mut clear = false;
        self.layer_selection.alpha_button = None;
        self.layer_selection.vector_button = None;
        egui::Window::new("Layer alpha and editable vector selections").open(&mut open).collapsible(false).default_width(450.0).show(ctx,|ui|{
            egui::ComboBox::from_id_salt("alpha-source").selected_text(label(d,e.source)).show_ui(ui,|ui|{for source in alpha::sources(d){ui.selectable_value(&mut e.source,source,label(d,source));}});
            egui::ComboBox::from_id_salt("layer-selection-combine").selected_text(format!("{:?}",e.combine)).show_ui(ui,|ui|{for op in Combine::ALL{ui.selectable_value(&mut e.combine,op,format!("{op:?}"));}});
            let r=ui.button("Apply layer alpha to area selection");self.layer_selection.alpha_button=Some(r.rect);if r.clicked(){action=Some(false)}
            ui.label("Uses raw layer-content alpha, including hidden/locked layer contents and pixels outside the canvas. Source pixels and ink stay unchanged. Add/subtract/intersect keep fractional coverage.");
            ui.separator();egui::ComboBox::from_id_salt("area-vector-layer").selected_text(e.layer.and_then(|id|d.scene.lines.layers.iter().find(|l|l.id==id)).map_or("Choose vector layer",|l|l.name.as_str())).show_ui(ui,|ui|{for layer in &d.scene.lines.layers{ui.selectable_value(&mut e.layer,Some(layer.id),&layer.name);}});
            ui.horizontal(|ui|{ui.radio_value(&mut e.kind,Kind::Points,"Select control points");ui.radio_value(&mut e.kind,Kind::Strokes,"Select intersecting whole strokes");});
            ui.checkbox(&mut e.width,"Include stroke width (otherwise centerline only)");let r=ui.button("Convert area to vector editing selection");self.layer_selection.vector_button=Some(r.rect);if r.clicked(){action=Some(true)}
            clear=ui.button("Clear vector editing selection").clicked();ui.label("Control points use nonzero area coverage. A crossing stroke can be selected with both ends outside. Empty areas clear on Replace; no area and hidden/locked vector layers are rejected. Editing selection is stored in the page and uses one Undo.");
            ui.checkbox(&mut self.workspaces.store.current.selection_outline,"Show area selection outline");ui.label("Hiding the outline keeps selection strength and painting restrictions active.");
        });
        self.canvas_view.selection_outline = self.workspaces.store.current.selection_outline;
        if !open {
            self.layer_selection.draft = None;
            return;
        }
        if let Some(v) = action
            && let Err(error) = self.apply_layer_selection(v)
        {
            self.status = Some(error);
        }
        if clear {
            let d = &self.documents[n];
            let fresh = self.layer_selection.draft.as_ref().is_some_and(|e| {
                (e.id, e.revision, e.frame) == (d.id, d.revision, d.current_frame)
            });
            if !fresh {
                self.status = Some("Page/frame changed; reopen layer selection".into());
                return;
            }
            match self.clear_vector_selection() {
                Ok(()) => {
                    if let Some(e) = &mut self.layer_selection.draft {
                        e.revision = self.documents[n].revision;
                    }
                }
                Err(error) => self.status = Some(error),
            }
        }
    }
}
fn label(d: &Document, s: Source) -> String {
    match s {
        Source::Raster(n) => d.layers.get(n).map_or("Missing raster".into(), |l| {
            format!("Raster {} · {}", n + 1, l.name)
        }),
        Source::Vector(id) => d
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == id)
            .map_or("Missing vector layer".into(), |l| {
                format!("Vector · {}", l.name)
            }),
        Source::Gradient(id) => format!("Gradient {id}"),
    }
}
/// Permanent selection feedback is a view overlay; neither renderer nor export receives it.
pub(super) fn paint(
    painter: &egui::Painter,
    d: &Document,
    view: drawer_core::view::View,
    rect: egui::Rect,
    ppp: f64,
    outline: bool,
) -> (usize, usize) {
    let project = |p| {
        let p = view.document_to_screen(
            p,
            [d.width as f64, d.height as f64],
            [rect.center().x as f64, rect.center().y as f64],
            ppp,
        );
        egui::pos2(p[0] as f32, p[1] as f32)
    };
    let mut edges = 0;
    let mut marks = 0;
    if outline {
        let segments = if let Some(mask) = &d.scene.selection_mask {
            mask.boundaries()
        } else if let Some(r) = d
            .scene
            .selection
            .filter(|r| r.min[0] < r.max[0] && r.min[1] < r.max[1])
        {
            let p = [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]];
            (0..4).map(|n| [p[n], p[(n + 1) % 4]]).collect()
        } else {
            vec![]
        };
        for [a, b] in segments {
            let a = project(d.scene.selection_point(a));
            let b = project(d.scene.selection_point(b));
            if a.is_finite()
                && b.is_finite()
                && egui::Rect::from_two_pos(a, b).expand(1.0).intersects(rect)
            {
                painter.line_segment([a, b], egui::Stroke::new(2.5, egui::Color32::BLACK));
                painter.line_segment([a, b], egui::Stroke::new(1.0, egui::Color32::WHITE));
                edges += 1;
            }
        }
    }
    let selection = &d.scene.lines.selection;
    if let Some(layer) = d
        .scene
        .lines
        .layers
        .iter()
        .find(|l| Some(l.id) == selection.layer && !l.hidden)
    {
        for id in &selection.paths {
            if let Some(path) = layer.paths.iter().find(|p| p.id == *id) {
                for (a, b) in path.segments() {
                    let a = project(d.scene.transform.map(a));
                    let b = project(d.scene.transform.map(b));
                    if a.is_finite()
                        && b.is_finite()
                        && egui::Rect::from_two_pos(a, b).expand(1.0).intersects(rect)
                    {
                        painter.line_segment(
                            [a, b],
                            egui::Stroke::new(1.5, egui::Color32::LIGHT_BLUE),
                        );
                        marks += 1;
                    }
                }
            }
        }
        for point in &selection.points {
            if let Some(p) = layer
                .paths
                .iter()
                .find(|p| p.id == point.path)
                .and_then(|p| p.control_point(point.point as usize))
            {
                let p = d.scene.transform.map(p);
                if p.iter().all(|v| v.is_finite() && v.abs() <= 1e12) {
                    let p = project(p);
                    if rect.expand(4.0).contains(p) {
                        painter.circle_filled(p, 3.5, egui::Color32::LIGHT_BLUE);
                        painter.circle_stroke(p, 3.5, egui::Stroke::new(1.0, egui::Color32::WHITE));
                        marks += 1;
                    }
                }
            }
        }
    }
    (edges, marks)
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_selection_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    let mut time = 0.0;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let source = app.documents[0].layers[0].clone();
    app.documents[0].layers.push(source);
    for y in 4..8 {
        for x in 4..8 {
            app.documents[0].layers[1].pixels[y * 16 + x] = [255, 255, 255, 128];
        }
    }
    drawer_core::selection_tools::rectangle(&mut app.documents[0], [1.0, 1.0], [2.0, 2.0])?;
    app.documents[0].scene.selection = None;
    app.documents[0].scene.selection_mask = None;
    app.documents[0].view.angle = 90.0;
    app.documents[0].view.zoom = 8.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "layer-selection-tools");
    app.layer_selection
        .draft
        .as_mut()
        .ok_or("Layer selection dialog missing")?
        .source = Source::Raster(1);
    let expected = alpha::prepare(&app.documents[0], Source::Raster(1), Combine::Replace)?;
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let rect = app
        .layer_selection
        .alpha_button
        .ok_or("Alpha Apply button missing")?;
    for pressed in [true, false] {
        step(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    if app.documents[0].scene != expected
        || app.documents[0].layers != before.layers
        || app.documents[0].history_counts().0 != history.0 + 1
    {
        return Err(format!("Alpha production Apply: {:?}", app.status));
    }
    let history = app.documents[0].history_counts();
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
    step(
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    if app.layer_selection.draft.is_some()
        || app.documents[0].history_counts() != history
        || app.canvas_view.selection_edges == 0
    {
        return Err("Layer selection close/visible contour".into());
    }
    let selected = app.documents[0].snapshot();
    app.execute_command(&ctx, "selection-outline");
    step(&mut app, &mut time, vec![]);
    if app.workspaces.store.current.selection_outline
        || app.canvas_view.selection_edges != 0
        || app.documents[0].snapshot() != selected
        || app.documents[0].history_counts() != history
    {
        return Err("Contour visibility changed selection/document".into());
    }
    app.set_tool(BrushTip::PixelPen);
    app.panels.settings.brush_size = 1.0;
    app.workspaces.store.current.pressure.tools[6].size = false;
    app.workspaces.store.current.pressure.tools[6].opacity = false;
    app.workspaces.store.current.input.profiles[6].minimum_size = 1.0;
    app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.0;
    app.workspaces.store.current.input.profiles[6]
        .pixel
        .pressure = false;
    app.choose_color([200, 40, 20, 255]);
    step(&mut app, &mut time, vec![]);
    let event = super::precision::pointer(
        &ctx,
        &app,
        [0.5, 6.5],
        drawer_core::pointer_input::Phase::Down,
    );
    app.drawing.virtual_events(vec![event], false);
    step(&mut app, &mut time, vec![]);
    let history = app.documents[0].history_counts();
    for _ in 0..2 {
        app.execute_command(&ctx, "selection-outline");
        if !app.drawing.pending()
            || app.documents[0].snapshot() != selected
            || app.documents[0].history_counts() != history
        {
            return Err("Toggling contour finalized pending input".into());
        }
    }
    let event = super::precision::pointer(
        &ctx,
        &app,
        [12.5, 6.5],
        drawer_core::pointer_input::Phase::Up,
    );
    app.drawing.virtual_events(vec![event], false);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].layers[0].pixel(16, 16, 6, 6) != [200, 40, 20, 128]
        || [0, 2, 10, 12]
            .into_iter()
            .any(|x| app.documents[0].layers[0].pixel(16, 16, x, 6)[3] != 0)
        || app.documents[0].scene.selection_mask != selected.scene.selection_mask
        || app.documents[0].history_counts().0 != history.0 + 1
    {
        return Err(format!(
            "Hidden outline lost selected painting restriction: {:?}",
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != selected {
        return Err("Hidden-contour painting Undo".into());
    }
    let layer = app.documents[0]
        .scene
        .lines
        .add_layer("Vector selection source")?;
    let path = app.documents[0].scene.lines.add_path(
        layer,
        vec![[-2.0, 6.5], [18.0, 6.5]],
        false,
        1.0,
        [20, 40, 60, 255],
    )?;
    app.execute_command(&ctx, "layer-selection-tools");
    let e = app.layer_selection.draft.as_mut().unwrap();
    e.layer = Some(layer);
    e.kind = Kind::Strokes;
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    let expected = vector::prepare(
        &app.documents[0],
        layer,
        Kind::Strokes,
        false,
        Combine::Replace,
    )?;
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let rect = app
        .layer_selection
        .vector_button
        .ok_or("Vector conversion Apply missing")?;
    for pressed in [true, false] {
        step(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    step(&mut app, &mut time, vec![]);
    if app.documents[0].scene != expected
        || app.documents[0].layers != before.layers
        || app.documents[0].history_counts().0 != history.0 + 1
        || app.documents[0].scene.lines.selection.paths != vec![path]
        || app.canvas_view.vector_selection_marks == 0
    {
        return Err(format!(
            "Vector conversion production Apply/feedback: {:?}",
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Vector conversion production Undo".into());
    }
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = app.documents[0].snapshot();
    if app.apply_layer_selection(true).is_ok() || app.documents[0].snapshot() != before {
        return Err("Stale vector conversion changed page".into());
    }
    app.layer_selection.draft = None;
    let workspace = app.capture_workspace();
    let file = root.join("hidden-outline-settings.json");
    workspace.export(&file).map_err(|e| e.to_string())?;
    if drawer_core::workspaces::Snapshot::read(&file)
        .map_err(|e| e.to_string())?
        .selection_outline
    {
        return Err("Hidden contour setting was not preserved".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn alpha_conversion_buttons_hidden_contour_paint_and_editing_feedback() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
