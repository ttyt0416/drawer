use super::*;
use drawer_core::{
    auto_shape::{self as core, Capture, Kind, Plan, Settings},
    pointer_input::{Phase, Source},
    shapes::Target,
};
pub(super) struct Monitor {
    capture: Capture,
    settings: Settings,
    source: Source,
    layer: usize,
    revision: u64,
    frame: u32,
    root: egui::Rect,
}
pub(super) struct VectorCapture {
    pub document: u64,
    pub view: u64,
    pub projection: Projection,
    revision: u64,
    frame: u32,
    root: egui::Rect,
    source: Source,
    layer: u64,
    width: f64,
    color: [u8; 4],
    capture: Capture,
    settings: Settings,
    assistance: drawer_core::rulers::Runtime,
    snapping: drawer_core::snapping::Settings,
    paint_points: Vec<Point>,
    pub preview: Option<Document>,
}
enum Original {
    Raster(Box<Session>),
    Vector(Box<VectorCapture>),
}
pub(super) struct Correction {
    pub document: u64,
    pub view: u64,
    projection: Projection,
    revision: u64,
    frame: u32,
    root: egui::Rect,
    source: Source,
    settings: Settings,
    target: Target,
    original: Original,
    released: bool,
    pub plan: Plan,
    key: Option<Plan>,
    pub preview: Option<Document>,
    error: Option<String>,
    pub apply: Option<egui::Rect>,
    pub keep: Option<egui::Rect>,
    pub cancel: Option<egui::Rect>,
}
impl Correction {
    fn raw(&self) -> &[Point] {
        match &self.original {
            Original::Raster(s) => &s.auto.as_ref().unwrap().capture.points,
            Original::Vector(v) => &v.capture.points,
        }
    }
    fn fresh(&self, ctx: &egui::Context, doc: &Document, settings: Settings) -> bool {
        self.document == doc.id
            && self.revision == doc.revision
            && self.frame == doc.current_frame
            && self.projection.view == doc.view
            && self.root == ctx.content_rect()
            && (self.projection.ppp - ctx.pixels_per_point() as f64).abs() < 1e-8
            && self.settings == settings
    }
    fn refresh(&mut self, doc: &Document, budget: usize) {
        if self.key.as_ref() == Some(&self.plan) {
            return;
        }
        self.key = Some(self.plan.clone());
        let result = (|| {
            let state = self.plan.prepare(doc, self.target)?;
            if state
                .memory_bytes()
                .saturating_mul(2)
                .saturating_add(doc.memory_bytes())
                .saturating_add(64 * 1024 * 1024)
                > budget
            {
                return Err("Shape preview exceeds workspace memory".into());
            }
            bucket::preview(doc, state)
        })();
        match result {
            Ok(p) => {
                self.preview = Some(p);
                self.error = None;
            }
            Err(e) => {
                self.preview = None;
                self.error = Some(e);
            }
        }
    }
}
impl Drawing {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn auto_monitor(
        &self,
        doc: &Document,
        root: egui::Rect,
        source: Source,
        layer: usize,
        p: Point,
        style: Style,
    ) -> Result<Option<Monitor>, String> {
        if !self.auto_settings.enabled || style.erases() {
            return Ok(None);
        }
        Ok(Some(Monitor {
            capture: Capture::new(p, self.frame_time)?,
            settings: self.auto_settings,
            source,
            layer,
            revision: doc.revision,
            frame: doc.current_frame,
            root,
        }))
    }
    pub(super) fn auto_sample(&mut self, pos: egui::Pos2) -> Result<(), String> {
        if let Some(s) = &mut self.active
            && let Some(m) = &mut s.auto
        {
            m.capture.push(
                s.projection.point(pos),
                self.frame_time,
                m.settings.motion_pixels * s.projection.ppp / s.projection.view.zoom,
            )?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_vector_auto(
        &mut self,
        doc: &Document,
        view: u64,
        projection: Projection,
        root: egui::Rect,
        source: Source,
        layer: u64,
        point: Point,
        style: Style,
    ) -> Result<(), String> {
        if style.erases() {
            return Err("Choose a drawing tool/color for vector shape conversion".into());
        }
        let mut color = style.color;
        color[3] = (color[3] as f64 * style.opacity).round() as u8;
        let width = style.shape.size.clamp(0.05, 4096.0);
        let mut assistance = drawer_core::rulers::Runtime::for_target(
            doc,
            drawer_core::line_tools::Target::Vector(layer),
        )?;
        assistance.configure_snapping(
            doc,
            self.snap_settings,
            projection.ppp / projection.view.zoom,
        )?;
        assistance.set_snap_disabled(self.snap_disabled);
        let projected = assistance.constrain(point);
        assistance.check_snapping()?;
        let state = core::freehand_vector(doc, layer, &[projected], width, color)?;
        if state
            .memory_bytes()
            .saturating_mul(3)
            .saturating_add(64 * 1024 * 1024)
            > self.fill_budget
        {
            return Err("Vector gesture exceeds workspace memory".into());
        }
        self.vector_auto = Some(Box::new(VectorCapture {
            document: doc.id,
            view,
            projection,
            revision: doc.revision,
            frame: doc.current_frame,
            root,
            source,
            layer,
            width,
            color,
            capture: Capture::new(point, self.frame_time)?,
            assistance,
            snapping: self.snap_settings,
            paint_points: vec![projected],
            settings: self.auto_settings,
            preview: Some(bucket::preview(doc, state)?),
        }));
        Ok(())
    }
    pub(crate) fn conversion_preview(&self, document: u64) -> Option<&Document> {
        self.correction
            .as_ref()
            .filter(|c| c.document == document)
            .and_then(|c| c.preview.as_ref())
            .or_else(|| {
                self.vector_auto
                    .as_ref()
                    .filter(|v| v.document == document)
                    .and_then(|v| v.preview.as_ref())
            })
    }
    pub(super) fn original_preview(&self, document: u64) -> Option<&Stroke> {
        let c = self
            .correction
            .as_ref()
            .filter(|c| c.document == document && c.preview.is_none())?;
        match &c.original {
            Original::Raster(s) => Some(&s.stroke),
            _ => None,
        }
    }
    pub(super) fn advance_auto_input(&mut self, ctx: &egui::Context, docs: &mut [Document]) {
        if self
            .active
            .as_ref()
            .and_then(|s| s.auto.as_ref())
            .is_some_and(|m| m.settings != self.auto_settings)
        {
            self.cancel();
            return;
        }
        if self.correction.as_ref().is_some_and(|c| {
            self.mode != super::super::precision::Mode::Paint
                || docs
                    .iter()
                    .find(|d| d.id == c.document)
                    .is_none_or(|d| !c.fresh(ctx, d, self.auto_settings))
        }) {
            self.cancel();
            return;
        }
        if self.vector_auto.as_ref().is_some_and(|v| {
            self.mode != super::super::precision::Mode::Paint
                || v.settings != self.auto_settings
                || v.snapping != self.snap_settings
                || v.root != ctx.content_rect()
                || (v.projection.ppp - ctx.pixels_per_point() as f64).abs() > 1e-8
                || docs.iter().find(|d| d.id == v.document).is_none_or(|d| {
                    d.revision != v.revision
                        || d.current_frame != v.frame
                        || d.view != v.projection.view
                })
        }) {
            self.cancel();
            return;
        }
        for i in 0..self.events.len() {
            if self.events[i].1 {
                continue;
            }
            let drawer_core::touch::Effect::Pointer(e) = self.events[i].0 else {
                continue;
            };
            if let Some(c) = &mut self.correction {
                if e.source == c.source && matches!(e.phase, Phase::Up | Phase::Cancel) {
                    self.events[i].1 = true;
                    if e.phase == Phase::Cancel && !c.released {
                        self.cancel();
                        return;
                    }
                    c.released = true;
                    self.ownership.clear();
                }
                continue;
            }
            let Some(mut v) = self.vector_auto.take() else {
                continue;
            };
            if e.source != v.source {
                self.vector_auto = Some(v);
                continue;
            }
            self.events[i].1 = true;
            if e.phase == Phase::Cancel {
                self.ownership.clear();
                continue;
            }
            let Some(d) = docs.iter_mut().find(|d| d.id == v.document) else {
                self.ownership.clear();
                continue;
            };
            let p = v
                .projection
                .point(egui::pos2(e.position[0] as f32, e.position[1] as f32));
            let result = (|| {
                v.capture.push(
                    p,
                    self.frame_time,
                    v.settings.motion_pixels * v.projection.ppp / v.projection.view.zoom,
                )?;
                v.assistance.set_snap_disabled(self.snap_disabled);
                let projected = v.assistance.constrain(p);
                v.assistance.check_snapping()?;
                if v.paint_points
                    .last()
                    .is_none_or(|q| (q[0] - projected[0]).hypot(q[1] - projected[1]) >= 0.25)
                {
                    if v.paint_points.len() == 4096 {
                        return Err("Vector ruler input point limit".into());
                    }
                    v.paint_points.push(projected);
                }
                let state = core::freehand_vector(d, v.layer, &v.paint_points, v.width, v.color)?;
                if e.phase == Phase::Up {
                    d.commit_state(state)?;
                    v.preview = None;
                } else {
                    v.preview = Some(bucket::preview(d, state)?);
                }
                Ok::<_, String>(())
            })();
            if let Err(err) = result {
                self.error = Some(err);
                self.ownership.clear();
                continue;
            }
            if e.phase == Phase::Up {
                self.ownership.clear();
            } else {
                self.vector_auto = Some(v);
            }
        }
    }
    pub(super) fn promote_auto(&mut self, ctx: &egui::Context, docs: &[Document]) {
        if self.correction.is_some() {
            return;
        }
        let raster = self.active.as_ref().is_some_and(|s| {
            s.auto
                .as_ref()
                .is_some_and(|m| m.capture.ready(self.frame_time, m.settings))
        });
        let vector = self
            .vector_auto
            .as_ref()
            .is_some_and(|v| v.capture.ready(self.frame_time, v.settings));
        if !raster && !vector {
            if self
                .active
                .as_ref()
                .and_then(|s| s.auto.as_ref())
                .is_some_and(|m| m.capture.waiting())
                || self
                    .vector_auto
                    .as_ref()
                    .is_some_and(|v| v.capture.waiting())
            {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            return;
        }
        let result = (|| -> Result<Correction, String> {
            if raster {
                let mut s = Box::new(self.active.take().unwrap());
                let m = s.auto.as_mut().unwrap();
                m.capture.consume();
                let raw = &m.capture.points;
                let style = s.stroke.style();
                let mut color = style.color;
                color[3] = (color[3] as f64 * style.opacity).round() as u8;
                let plan = Plan::from_points(
                    raw,
                    core::recognize(raw)?,
                    style.shape.size.clamp(0.05, 4096.0),
                    color,
                )?;
                Ok(Correction {
                    document: s.stroke.document,
                    view: s.view,
                    projection: s.projection,
                    revision: m.revision,
                    frame: m.frame,
                    root: m.root,
                    source: m.source,
                    settings: m.settings,
                    target: Target::Raster(m.layer),
                    original: Original::Raster(s),
                    released: false,
                    plan,
                    key: None,
                    preview: None,
                    error: None,
                    apply: None,
                    keep: None,
                    cancel: None,
                })
            } else {
                let mut v = self.vector_auto.take().unwrap();
                v.capture.consume();
                let raw = &v.capture.points;
                let plan = Plan::from_points(raw, core::recognize(raw)?, v.width, v.color)?;
                Ok(Correction {
                    document: v.document,
                    view: v.view,
                    projection: v.projection,
                    revision: v.revision,
                    frame: v.frame,
                    root: v.root,
                    source: v.source,
                    settings: v.settings,
                    target: Target::Vector(Some(v.layer)),
                    original: Original::Vector(v),
                    released: false,
                    plan,
                    key: None,
                    preview: None,
                    error: None,
                    apply: None,
                    keep: None,
                    cancel: None,
                })
            }
        })();
        match result {
            Ok(mut c) => {
                if let Some(doc) = docs.iter().find(|d| d.id == c.document)
                    && c.fresh(ctx, doc, self.auto_settings)
                {
                    c.refresh(doc, self.fill_budget);
                    self.correction = Some(Box::new(c));
                    ctx.request_repaint();
                }
            }
            Err(e) => {
                self.error = Some(e);
                self.ownership.clear();
            }
        }
    }
    fn keep_original(&mut self, ctx: &egui::Context, docs: &mut [Document]) -> Result<(), String> {
        let c = self.correction.take().ok_or("Shape correction is closed")?;
        let d = docs
            .iter_mut()
            .find(|d| d.id == c.document)
            .ok_or("Source page was closed")?;
        if !c.fresh(ctx, d, self.auto_settings) {
            return Err("Page/frame/view changed; correction cancelled".into());
        }
        match c.original {
            Original::Raster(s) => {
                if c.released {
                    self.commit_session(*s, d);
                } else {
                    self.active = Some(*s);
                }
            }
            Original::Vector(v) => {
                if c.released {
                    d.commit_state(core::freehand_vector(
                        d,
                        v.layer,
                        &v.paint_points,
                        v.width,
                        v.color,
                    )?)?;
                } else {
                    self.vector_auto = Some(v);
                }
            }
        }
        Ok(())
    }
    fn apply_correction(
        &mut self,
        ctx: &egui::Context,
        docs: &mut [Document],
    ) -> Result<(), String> {
        let c = self
            .correction
            .as_ref()
            .ok_or("Shape correction is closed")?;
        let d = docs
            .iter_mut()
            .find(|d| d.id == c.document)
            .ok_or("Source page was closed")?;
        if !c.fresh(ctx, d, self.auto_settings) {
            return Err("Page/frame/view changed; correction cancelled".into());
        }
        let state = c.plan.prepare(d, c.target)?;
        if state
            .memory_bytes()
            .saturating_mul(2)
            .saturating_add(d.memory_bytes())
            .saturating_add(64 * 1024 * 1024)
            > self.fill_budget
        {
            return Err("Converted shape exceeds workspace memory".into());
        }
        d.commit_state(state)?;
        self.correction = None;
        self.ownership.clear();
        ctx.request_repaint();
        Ok(())
    }
}
impl Drawer {
    pub(crate) fn auto_shape_controls(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Hold stroke end to edit shape",|ui|{let s=&mut self.workspaces.store.current.auto_shape;let before=*s;ui.checkbox(&mut s.enabled,"Enable stroke-to-shape correction");ui.add(egui::Slider::new(&mut s.hold_ms,0..=5000).text("Hold milliseconds"));ui.add(egui::Slider::new(&mut s.motion_pixels,0.0..=64.0).text("Motion tolerance (screen pixels)"));if *s!=before{self.finish_stroke();}ui.label("Hold a drawing stroke to preview a shape. Change candidate and control points before applying. Cancel correction restores the original freehand; Cancel whole stroke discards it. Erasing strokes keep their normal behavior.");});
    }
    pub(crate) fn auto_shape_window(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.drawing.correction.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == c.document) else {
            return;
        };
        let d = &self.documents[n];
        let mut open = true;
        let mut apply = false;
        let mut keep = false;
        let mut cancel = false;
        egui::Window::new("Convert current stroke to shape").open(&mut open).collapsible(false).resizable(false).default_width(460.0).min_width(460.0).show(ctx,|ui|{
        let previous=c.plan.kind;ui.horizontal_wrapped(|ui|{for kind in Kind::ALL{ui.selectable_value(&mut c.plan.kind,kind,format!("{kind:?}"));}});
        if c.plan.kind!=previous{match Plan::from_points(c.raw(),c.plan.kind,c.plan.width,c.plan.color){Ok(plan)=>c.plan=plan,Err(e)=>c.error=Some(e)}}
        ui.label("Edit document-space control points:");egui::ScrollArea::vertical().id_salt("auto-shape-controls").max_height(150.0).show(ui,|ui|{for(i,p)in c.plan.controls.iter_mut().enumerate(){ui.horizontal(|ui|{ui.label(format!("{}",i+1));for v in p{ui.add(egui::DragValue::new(v).range(-1e9..=1e9));}});}});
        if c.plan.kind==Kind::Polygon{ui.horizontal(|ui|{if ui.add_enabled(c.plan.controls.len()<128,egui::Button::new("Add control point")).clicked(){c.plan.controls.push(*c.plan.controls.last().unwrap());}
if ui.add_enabled(c.plan.controls.len()>3,egui::Button::new("Remove last point")).clicked(){c.plan.controls.pop();}});}
        ui.add(egui::Slider::new(&mut c.plan.width,0.05..=4096.0).logarithmic(true).text("Line width (document pixels)"));if matches!(c.plan.kind,Kind::Rectangle|Kind::Triangle|Kind::Polygon){ui.add(egui::Slider::new(&mut c.plan.corner,0.0..=1024.0).text("Round corners (page pixels)"));}super::super::edit::rgba(ui,&mut c.plan.color);
        c.refresh(d,self.drawing.fill_budget);if let Some(err)=&c.error{ui.label(err);}
        ui.label("Preview is separate from the document. Apply replaces this freehand stroke with one editable shape / raster result and one Undo entry.");
        ui.horizontal_wrapped(|ui|{let r=ui.add_enabled(c.preview.is_some(),egui::Button::new("Apply converted shape"));c.apply=Some(r.rect);apply=r.clicked();let r=ui.button("Cancel correction / keep freehand");c.keep=Some(r.rect);keep=r.clicked();let r=ui.button("Cancel whole stroke");c.cancel=Some(r.rect);cancel=r.clicked();});
    });

        if !open || cancel {
            self.drawing.ownership.clear();
            return;
        }
        self.drawing.correction = Some(c);
        let result = if apply {
            self.drawing.apply_correction(ctx, &mut self.documents)
        } else if keep {
            self.drawing.keep_original(ctx, &mut self.documents)
        } else {
            Ok(())
        };
        if let Err(err) = result {
            self.status = Some(err);
        }
    }
}

pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::auto_shape_verify::verify(root)?;
    for pen in [false, true] {
        for vector in [false, true] {
            for action in 0..3 {
                let ctx = egui::Context::default();
                let mut app = Drawer::new(&ctx, None);
                app.simple_view = true;
                app.create_canvas(CanvasSpec {
                    width: 64,
                    height: 64,
                    dpi: 300.0,
                    background: Background::Transparent,
                })
                .map_err(|e| e.to_string())?;
                if vector {
                    let layer = app.documents[0].scene.lines.add_layer("Stroke target")?;
                    app.panels.targets.insert(
                        app.documents[0].id,
                        drawer_core::thumbnail::Target::Vector(layer),
                    );
                }
                app.set_tool(BrushTip::PixelPen);
                app.choose_color([200, 40, 20, 255]);
                app.panels.settings.brush_size = 2.0;
                app.workspaces.store.current.pressure.tools[6].size = false;
                app.workspaces.store.current.pressure.tools[6].opacity = false;
                app.workspaces.store.current.input.profiles[6]
                    .pixel
                    .pressure = false;
                app.workspaces.store.current.auto_shape = Settings {
                    enabled: true,
                    hold_ms: 500,
                    motion_pixels: 3.0,
                };
                app.documents[0].view.zoom = 2.0;
                app.documents[0].view.angle = 90.0;
                let frame = |app: &mut Drawer, t, events| {
                    super::super::input_replay::frame(&ctx, app, t, events)
                };
                let pointer = |app: &mut Drawer, t, p, phase| {
                    let mut e = super::super::precision::pointer(&ctx, app, p, phase);
                    if pen {
                        e.source = Source {
                            kind: drawer_core::pointer_input::Kind::Pen,
                            device: 113,
                            contact: 1,
                        };
                        e.pressure = Some(0.5);
                    }
                    app.drawing.virtual_events(vec![e], pen);
                    frame(app, t, vec![]);
                };
                frame(&mut app, 0.0, vec![]);
                frame(&mut app, 0.01, vec![]);
                let before = app.documents[0].snapshot();
                let history = app.documents[0].history_counts();
                let points = [
                    [12.0, 12.0],
                    [48.0, 12.0],
                    [48.0, 48.0],
                    [12.0, 48.0],
                    [12.0, 12.0],
                ];
                for (i, p) in points.into_iter().enumerate() {
                    pointer(
                        &mut app,
                        0.1 + i as f64 * 0.1,
                        p,
                        if i == 0 { Phase::Down } else { Phase::Move },
                    );
                }
                frame(&mut app, 0.999999, vec![]);
                if app.drawing.correction.is_some() || !app.drawing.pending() {
                    return Err(format!(
                        "Shape hold triggered before threshold {pen}/{vector}: {:?}",
                        app.status
                    ));
                }
                frame(&mut app, 1.0, vec![]);
                frame(&mut app, 1.01, vec![]);
                let c = app.drawing.correction.as_ref().ok_or_else(|| {
                    format!(
                        "Shape hold did not trigger {pen}/{vector}: {:?}",
                        app.status
                    )
                })?;
                if c.plan.kind != Kind::Rectangle
                    || app.documents[0].snapshot() != before
                    || app.documents[0].history_counts() != history
                {
                    return Err("Hold preview changed source or lost proposal".into());
                }
                let raw = c.raw().to_vec();
                let raster_preview = if !vector {
                    let Original::Raster(s) = &c.original else {
                        unreachable!()
                    };
                    Some(
                        (0..64)
                            .flat_map(|y| (0..64).map(move |x| (x, y)))
                            .map(|(x, y)| {
                                s.stroke
                                    .pixel(&app.documents[0], 0, &app.documents[0].layers[0], x, y)
                                    .unwrap_or([0; 4])
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                };
                pointer(&mut app, 1.02, [12.0, 12.0], Phase::Up);
                if !app.drawing.correction.as_ref().is_some_and(|c| c.released) {
                    return Err("Release finalized/discarded correction draft".into());
                }
                if action == 0 {
                    let c = app.drawing.correction.as_mut().unwrap();
                    c.plan = Plan::from_points(&raw, Kind::Triangle, 2.0, [200, 40, 20, 255])?;
                    c.plan.controls[0] = [31.0, 10.0];
                    c.plan.corner = 2.0;
                }
                frame(&mut app, 1.03, vec![]);
                frame(&mut app, 1.04, vec![]);
                let c = app.drawing.correction.as_ref().unwrap();
                let expected = if action == 0 {
                    Some(c.plan.prepare(&app.documents[0], c.target)?)
                } else {
                    None
                };
                let r = match action {
                    0 => c.apply,
                    1 => c.keep,
                    _ => c.cancel,
                }
                .ok_or("Correction action button missing")?;
                for (t, pressed) in [(1.05, true), (1.06, false)] {
                    frame(
                        &mut app,
                        t,
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
                frame(&mut app, 1.07, vec![]);
                if app.drawing.pending() {
                    return Err(format!(
                        "Correction action did not finish {pen}/{vector}/{action}: {:?}",
                        app.status
                    ));
                }
                if action == 2 {
                    if app.documents[0].snapshot() != before
                        || app.documents[0].history_counts() != history
                    {
                        return Err("Cancel whole stroke changed page/history".into());
                    }
                    continue;
                }
                if app.documents[0].history_counts().0 != history.0 + 1 {
                    return Err(format!(
                        "Freehand-to-shape action history pen={pen} vector={vector} action={action} history={:?}/{history:?} painted={} status={:?} error={:?}",
                        app.documents[0].history_counts(),
                        app.documents[0].layers[0]
                            .pixels
                            .iter()
                            .filter(|p| p[3] > 0)
                            .count(),
                        app.status,
                        app.drawing.error
                    ));
                }
                if let Some(expected) = expected {
                    if app.documents[0].snapshot() != expected {
                        return Err("Edited candidate Apply did not replace original stroke".into());
                    }
                } else if vector {
                    if app.documents[0].scene.lines.layers[0].paths[0].points != raw
                        || app.documents[0].scene.lines.layers[0].paths[0]
                            .definition
                            .is_some()
                    {
                        return Err("Cancel correction did not restore vector freehand".into());
                    }
                } else if Some(&app.documents[0].layers[0].pixels) != raster_preview.as_ref() {
                    return Err("Cancel correction did not restore raster freehand pixels".into());
                }
                let file = root.join(format!("auto-shape-input-{pen}-{vector}-{action}.drawer"));
                drawer_core::files::save(&mut app.documents[0], &file)
                    .map_err(|e| e.to_string())?;
                if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                    .map_err(|e| e.to_string())?
                    .snapshot()
                    != app.documents[0].snapshot()
                {
                    return Err("Production converted/freehand file reopen".into());
                }
                if !app.documents[0].undo() || app.documents[0].snapshot() != before {
                    return Err("Production conversion/freehand single Undo".into());
                }
            }
        }
    }
    edge_smoke()?;
    Ok(())
}
fn edge_smoke() -> Result<(), String> {
    for vector in [false, true] {
        for scenario in 0..3 {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            app.create_canvas(CanvasSpec {
                width: 64,
                height: 64,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            if vector {
                let id = app.documents[0].scene.lines.add_layer("Edge target")?;
                app.panels.targets.insert(
                    app.documents[0].id,
                    drawer_core::thumbnail::Target::Vector(id),
                );
            }
            app.set_tool(BrushTip::PixelPen);
            app.choose_color([200, 40, 20, 255]);
            app.panels.settings.brush_size = 2.0;
            app.workspaces.store.current.pressure.tools[6].size = false;
            app.workspaces.store.current.pressure.tools[6].opacity = false;
            app.workspaces.store.current.input.profiles[6]
                .pixel
                .pressure = false;
            app.workspaces.store.current.auto_shape = Settings {
                enabled: true,
                hold_ms: 100,
                motion_pixels: 3.0,
            };
            let frame = |app: &mut Drawer, t, events| {
                super::super::input_replay::frame(&ctx, app, t, events)
            };
            let pointer = |app: &mut Drawer, t, p, phase| {
                let mut e = super::super::precision::pointer(&ctx, app, p, phase);
                e.source = Source {
                    kind: drawer_core::pointer_input::Kind::Pen,
                    device: 114,
                    contact: 1,
                };
                e.pressure = Some(0.5);
                app.drawing.virtual_events(vec![e], true);
                frame(app, t, vec![]);
            };
            frame(&mut app, 0.0, vec![]);
            frame(&mut app, 0.01, vec![]);
            let before = app.documents[0].snapshot();
            pointer(&mut app, 0.1, [8.0, 8.0], Phase::Down);
            if scenario == 0 {
                pointer(&mut app, 0.3, [8.0, 8.0], Phase::Up);
                if app.drawing.pending() || app.documents[0].history_counts().0 != 1 {
                    return Err("Auto-enabled dot was lost/converted".into());
                }
                continue;
            }
            pointer(&mut app, 0.2, [40.0, 40.0], Phase::Move);
            if scenario == 1 {
                pointer(&mut app, 0.3, [40.0, 40.0], Phase::Up);
                if !app.drawing.correction.as_ref().is_some_and(|c| c.released)
                    || app.documents[0].snapshot() != before
                {
                    return Err("Release at hold threshold bypassed correction".into());
                }
                app.drawing.cancel();
                continue;
            }
            frame(&mut app, 0.3, vec![]);
            frame(&mut app, 0.31, vec![]);
            frame(&mut app, 0.32, vec![]);
            let r = app
                .drawing
                .correction
                .as_ref()
                .and_then(|c| c.keep)
                .ok_or("Held correction keep button missing")?;
            for (t, pressed) in [(0.33, true), (0.34, false)] {
                frame(
                    &mut app,
                    t,
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
            frame(&mut app, 0.5, vec![]);
            if app.drawing.correction.is_some()
                || !app.drawing.pending()
                || app.documents[0].snapshot() != before
            {
                return Err(
                    "Keep while pen held failed to restore unchanged freehand / retriggered".into(),
                );
            }
            frame(
                &mut app,
                0.51,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            if app.drawing.pending()
                || app.documents[0].snapshot() != before
                || app.documents[0].history_counts().0 != 0
            {
                return Err("Whole stroke Escape after correction cancel changed page".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn hold_shape_mouse_pen_outputs_controls_keep_cancel_and_single_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
