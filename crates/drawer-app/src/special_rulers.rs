use super::*;
use drawer_core::{
    line_tools::Target,
    ruler_scope::{self, Display, Scope},
    rulers::{self, Data, Guide, Kind},
    scene::{Affine, Point},
    special_rulers::{Marks, Perspective, Spec},
};
const LABELS: [&str; 11] = [
    "Parallel line",
    "Parallel curve",
    "Multiple curve",
    "Radial line",
    "Radial curve",
    "Concentric circle",
    "Guide",
    "Equal division",
    "Golden ratio",
    "Editable perspective",
    "Fisheye",
];
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    follow: Option<Box<Follow>>,
    movement: Option<Box<Movement>>,
    apply: Option<egui::Rect>,
    follow_apply: Option<egui::Rect>,
    move_apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    pub data: Data,
    pub selected: Option<u64>,
    kind: usize,
    curve: usize,
    vanishing: u64,
    anchor: Point,
}
struct Follow {
    id: u64,
    revision: u64,
    frame: u32,
    guide: u64,
    part: usize,
    target: drawer_core::shapes::Target,
    width: f64,
    color: [u8; 4],
}
struct Movement {
    id: u64,
    revision: u64,
    frame: u32,
    target: Target,
    delta: Point,
}
fn target(app: &Drawer, d: &Document) -> Target {
    match app
        .panels
        .targets
        .get(&d.id)
        .copied()
        .unwrap_or(drawer_core::thumbnail::Target::Raster(0))
    {
        drawer_core::thumbnail::Target::Raster(i) => Target::Raster(i),
        drawer_core::thumbnail::Target::Vector(id) => Target::Vector(id),
        _ => Target::PageObjects,
    }
}
fn shape_target(t: Target) -> Result<drawer_core::shapes::Target, String> {
    match t {
        Target::Raster(i) => Ok(drawer_core::shapes::Target::Raster(i)),
        Target::Vector(id) => Ok(drawer_core::shapes::Target::Vector(Some(id))),
        _ => Err("Choose a raster or vector layer".into()),
    }
}
fn preset(doc: &Document, kind: usize) -> Spec {
    let inverse = doc.scene.transform.inverse();
    let c = inverse.map([doc.width as f64 / 2.0, doc.height as f64 / 2.0]);
    let scale = doc.scene.transform.0[0]
        .hypot(doc.scene.transform.0[1])
        .max(1e-6);
    let r = (doc.width.min(doc.height) as f64 / 3.0 / scale).max(0.25);
    let a = [c[0] - r, c[1]];
    let b = [c[0] + r, c[1]];
    let points = vec![a, [c[0], c[1] - r], b];
    match kind {
        0 => Spec::ParallelLine { a, b },
        1 => Spec::ParallelCurve { points },
        2 => Spec::MultipleCurve {
            curves: vec![
                points.clone(),
                points.iter().map(|p| [p[0], p[1] + r]).collect(),
            ],
        },
        3 => Spec::RadialLine {
            center: c,
            guide: b,
        },
        4 => Spec::RadialCurve {
            center: c,
            points: vec![c, [c[0] + r / 2.0, c[1] - r], b],
        },
        5 => Spec::ConcentricCircle {
            center: c,
            radius: r,
        },
        6 => Spec::Guide { a, b },
        7 => Spec::Division {
            a,
            b,
            marks: Marks::Equal(4),
            snap_ticks: false,
        },
        8 => Spec::Division {
            a,
            b,
            marks: Marks::Golden(2),
            snap_ticks: false,
        },
        _ => {
            let mut p = Perspective {
                center: c,
                radius: r,
                horizon: c[1],
                fisheye: kind == 10,
                ..Default::default()
            };
            p.points[0].point = a;
            p.points[1].point = b;
            Spec::Perspective(p)
        }
    }
}
impl Draft {
    fn select(&mut self, id: u64) {
        self.selected = Some(id);
        self.data.active = Some(id);
        self.curve = 0;
        if let Some(Guide {
            kind: Kind::Special(Spec::Perspective(p)),
            ..
        }) = self.data.guides.iter().find(|g| g.id == id)
        {
            self.vanishing = p.points.first().map_or(0, |p| p.id);
            self.anchor = [p.center[0], p.center[1] + p.radius / 2.0];
        }
    }
}
impl Drawer {
    pub(super) fn begin_special_rulers(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        self.begin_special_rulers_with(d.id, d.revision, d.scene.rulers.clone())
    }
    pub(super) fn begin_special_rulers_with(
        &mut self,
        id: u64,
        revision: u64,
        data: Data,
    ) -> Result<(), String> {
        let d = self
            .documents
            .iter()
            .find(|d| d.id == id)
            .ok_or("Source page was closed")?;
        let selected = data.active.or_else(|| data.guides.first().map(|g| g.id));
        let mut draft = Draft {
            id,
            revision,
            frame: d.current_frame,
            data,
            selected: None,
            kind: 0,
            curve: 0,
            vanishing: 0,
            anchor: [0.0; 2],
        };
        if let Some(id) = selected {
            draft.select(id);
        }
        self.special_rulers.draft = Some(Box::new(draft));
        Ok(())
    }
    fn apply_special_rulers(&mut self) -> Result<(), String> {
        let e = self
            .special_rulers
            .draft
            .as_ref()
            .ok_or("Ruler draft is closed")?;
        let d = self
            .documents
            .iter_mut()
            .find(|d| d.id == e.id && d.revision == e.revision && d.current_frame == e.frame)
            .ok_or("Page/frame changed; reopen rulers")?;
        let mut data = e.data.clone();
        for g in &mut data.guides {
            g.name = g.name.trim().into();
        }
        rulers::apply(d, data)?;
        self.special_rulers.draft = None;
        Ok(())
    }
    pub(super) fn special_ruler_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.special_rulers.draft.take() else {
            return;
        };
        let Some(n) = self.documents.iter().position(|d| d.id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = d.revision == e.revision && d.current_frame == e.frame;
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Special rulers and layer scope").open(&mut open).collapsible(false).default_width(510.0).min_width(510.0).default_pos(egui::pos2(80.0,40.0)).resizable(false).show(ctx,|ui|{
            ui.label(format!("{} · frame {}",d.title,e.frame+1));
            egui::ScrollArea::vertical().id_salt("special-ruler-body").max_height(350.0).auto_shrink([false,false]).show(ui,|ui|{
            ui.horizontal(|ui|{
                egui::ComboBox::from_id_salt("special-ruler-kind").selected_text(LABELS[e.kind]).show_ui(ui,|ui|{for(i,label)in LABELS.iter().enumerate(){ui.selectable_value(&mut e.kind,i,*label);}});
                if ui.add_enabled(e.data.guides.len()<32,egui::Button::new("Add ruler to draft")).clicked(){match e.data.add(LABELS[e.kind],Kind::Special(preset(d,e.kind))){Ok(id)=>e.select(id),Err(err)=>self.status=Some(err)}}
            });
            let mut choose=None;let mut remove=None;
            egui::ScrollArea::vertical().id_salt("special-ruler-list").max_height(100.0).show(ui,|ui|{for g in &e.data.guides{ui.push_id(g.id,|ui|{ui.horizontal(|ui|{if ui.selectable_label(e.selected==Some(g.id),format!("{} · {}",g.name,g.kind.label())).clicked(){choose=Some(g.id);}
if ui.button("Delete ruler").clicked(){remove=Some(g.id);}});});}});
            if let Some(id)=choose{e.select(id);}
            if let Some(id)=remove{e.data.guides.retain(|g|g.id!=id);if e.data.active==Some(id){e.data.active=None;}
if e.selected==Some(id){e.selected=None;}}
            if let Some(id)=e.selected && let Some(g)=e.data.guides.iter_mut().find(|g|g.id==id){
                super::edit::text(ui,&mut g.name,"special-ruler-name");ui.checkbox(&mut g.enabled,"Enable snapping for this ruler");
                scope_controls(ui,d,g);
                let transform=ruler_scope::transform(d,g).unwrap_or(d.scene.transform);
                let mut error=None;geometry_controls(ui,&mut g.kind,transform,&mut e.curve,&mut e.vanishing,&mut e.anchor,&mut error);
                if let Some(err)=error{self.status=Some(err);}
                let lines=rulers::lines(g,transform,[d.width as f64,d.height as f64]);mini_preview(ui,d,&lines);
            }
            });
            let valid=e.data.valid()&&fresh;
            if !fresh{ui.label("Page/frame changed; cancel and reopen this draft.");}else if !valid{ui.label("Correct invalid geometry, name, range or references before applying.");}
            ui.label("Ruler changes have their own Undo. Follow saved ruler and Move layer with rulers are separate commands. Folder scopes can keep each cut's raster layers separate.");
            ui.horizontal(|ui|{let r=ui.add_enabled(valid,egui::Button::new("Apply special rulers"));self.special_rulers.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.special_rulers.draft = Some(e);
        }
        if apply && let Err(err) = self.apply_special_rulers() {
            self.status = Some(err);
        }
    }
    pub(super) fn begin_follow_ruler(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let guide = d
            .scene
            .rulers
            .active
            .ok_or("Select an active ruler first")?;
        self.special_rulers.follow = Some(Box::new(Follow {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            guide,
            part: 0,
            target: shape_target(target(self, d))?,
            width: self.panels.settings.brush_size,
            color: self.references.session.workspace.color,
        }));
        Ok(())
    }
    fn apply_follow_ruler(&mut self) -> Result<(), String> {
        let e = self
            .special_rulers
            .follow
            .as_ref()
            .ok_or("Follow ruler is closed")?;
        let used = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        let d = self
            .documents
            .iter_mut()
            .find(|d| d.id == e.id && d.revision == e.revision && d.current_frame == e.frame)
            .ok_or("Page/frame changed; reopen follow ruler")?;
        let state = ruler_scope::follow_part(d, e.guide, e.target, e.width, e.color, e.part)?;
        if state.memory_bytes().saturating_mul(2).saturating_add(used) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Ruler stroke exceeds workspace memory".into());
        }
        d.commit_state(state)?;
        self.special_rulers.follow = None;
        Ok(())
    }
    pub(super) fn follow_ruler_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.special_rulers.follow.take() else {
            return;
        };
        let Some(d) = self.documents.iter().find(|d| d.id == e.id) else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Draw along saved ruler").open(&mut open).collapsible(false).default_width(440.0).min_width(440.0).show(ctx,|ui|{
            egui::ComboBox::from_id_salt("follow-ruler-source").selected_text(format!("Ruler {}",e.guide)).show_ui(ui,|ui|{for g in &d.scene.rulers.guides{if ui.selectable_value(&mut e.guide,g.id,&g.name).changed(){e.part=0;}}});
            if let Some(g)=d.scene.rulers.guides.iter().find(|g|g.id==e.guide){let count=match &g.kind{Kind::Special(Spec::MultipleCurve{curves})=>curves.len(),Kind::Special(Spec::Perspective(p))=>p.guides.len(),_=>1};ui.add(egui::Slider::new(&mut e.part,0..=count.saturating_sub(1)).text("Curve / guide index"));if let Ok((points,closed))=ruler_scope::path(d,g.id,e.part){let mut lines=points.windows(2).map(|p|[p[0],p[1]]).collect::<Vec<_>>();if closed&&!points.is_empty(){lines.push([*points.last().unwrap(),points[0]]);}mini_preview(ui,d,&lines);}}
            ui.label(format!("Output: {:?}",e.target));ui.add(egui::Slider::new(&mut e.width,0.05..=4096.0).logarithmic(true).text("Document line width"));super::edit::rgba(ui,&mut e.color);
            let fresh=d.revision==e.revision&&d.current_frame==e.frame;let result=if fresh{ruler_scope::follow_part(d,e.guide,e.target,e.width,e.color,e.part)}else{Err("Page/frame changed; reopen this command".into())};
            if let Err(err)=&result{ui.label(err);}ui.label("Draws one finite base path with one Undo. Scope, target lock, selection and drawing limits apply.");
            ui.horizontal(|ui|{let r=ui.add_enabled(result.is_ok(),egui::Button::new("Draw ruler path"));self.special_rulers.follow_apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.special_rulers.follow = Some(e);
        }
        if apply && let Err(err) = self.apply_follow_ruler() {
            self.status = Some(err);
        }
    }
    pub(super) fn begin_ruler_layer_move(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = target(self, d);
        shape_target(target)?;
        self.special_rulers.movement = Some(Box::new(Movement {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            delta: [0.0; 2],
        }));
        Ok(())
    }
    fn apply_ruler_layer_move(&mut self) -> Result<(), String> {
        let e = self
            .special_rulers
            .movement
            .as_ref()
            .ok_or("Layer movement is closed")?;
        let d = self
            .documents
            .iter_mut()
            .find(|d| d.id == e.id && d.revision == e.revision && d.current_frame == e.frame)
            .ok_or("Page/frame changed; reopen layer movement")?;
        d.commit_state(ruler_scope::translate_layer(d, e.target, e.delta)?)?;
        self.special_rulers.movement = None;
        Ok(())
    }
    pub(super) fn ruler_layer_move_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.special_rulers.movement.as_mut() else {
            return;
        };
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Move layer with linked rulers").open(&mut open).collapsible(false).show(ctx,|ui|{ui.label(format!("Layer: {:?}. Raster movement rounds to pixels; vector movement keeps decimals.",e.target));ui.horizontal(|ui|{ui.label("X / Y");for v in &mut e.delta{ui.add(egui::DragValue::new(v).range(-1e6..=1e6));}});ui.horizontal(|ui|{let r=ui.button("Move layer and rulers");self.special_rulers.move_apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});});
        if !open || cancel {
            self.special_rulers.movement = None;
        } else if apply && let Err(err) = self.apply_ruler_layer_move() {
            self.status = Some(err);
        }
    }
}
fn point_control(ui: &mut egui::Ui, p: &mut Point, t: Affine) -> bool {
    let mut doc = t.map(*p);
    let mut changed = false;
    ui.horizontal(|ui| {
        for v in &mut doc {
            changed |= ui.add(egui::DragValue::new(v).range(-1e9..=1e9)).changed();
        }
    });
    if changed {
        *p = t.inverse().map(doc);
    }
    changed
}
fn curve_controls(ui: &mut egui::Ui, points: &mut Vec<Point>, t: Affine) {
    for (i, p) in points.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.label(format!("Control {}", i + 1));
            point_control(ui, p, t);
        });
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(points.len() < 32, egui::Button::new("Add curve control"))
            .clicked()
        {
            let p = *points.last().unwrap_or(&[0.0; 2]);
            points.push([p[0] + 8.0, p[1]]);
        }
        if ui
            .add_enabled(
                points.len() > 2,
                egui::Button::new("Remove last curve control"),
            )
            .clicked()
        {
            points.pop();
        }
    });
}
fn scope_controls(ui: &mut egui::Ui, d: &Document, g: &mut Guide) {
    egui::ComboBox::from_id_salt("ruler-display-scope")
        .selected_text(format!("Display: {:?}", g.display))
        .show_ui(ui, |ui| {
            for display in [Display::All, Display::AppliedLayers, Display::Hidden] {
                ui.selectable_value(&mut g.display, display, format!("{display:?}"));
            }
        });
    egui::ComboBox::from_id_salt("ruler-application-scope")
        .selected_text(format!("Apply: {:?}", g.scope))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut g.scope, Scope::All, "All layers");
            for (i, l) in d.layers.iter().enumerate() {
                ui.selectable_value(
                    &mut g.scope,
                    Scope::Layer(Target::Raster(i)),
                    format!("Raster {}: {}", i + 1, l.name),
                );
            }
            for l in &d.scene.lines.layers {
                ui.selectable_value(
                    &mut g.scope,
                    Scope::Layer(Target::Vector(l.id)),
                    format!("Vector: {}", l.name),
                );
            }
            for (i, f) in d.scene.paper.folders.iter().enumerate() {
                ui.selectable_value(
                    &mut g.scope,
                    Scope::Folder(i as u16),
                    format!("Folder/cut group: {}", f.name),
                );
            }
        });
    egui::ComboBox::from_id_salt("ruler-layer-link")
        .selected_text(format!("Linked layer: {:?}", g.link))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut g.link, None, "No layer link");
            for (i, l) in d.layers.iter().enumerate() {
                ui.selectable_value(
                    &mut g.link,
                    Some(Target::Raster(i)),
                    format!("Raster {}: {}", i + 1, l.name),
                );
            }
            for l in &d.scene.lines.layers {
                ui.selectable_value(
                    &mut g.link,
                    Some(Target::Vector(l.id)),
                    format!("Vector: {}", l.name),
                );
            }
        });
    let mut range = g.frames.is_some();
    if ui
        .checkbox(&mut range, "Limit to animation frame range")
        .changed()
    {
        g.frames = range.then_some([d.current_frame, d.current_frame]);
    }
    if let Some(r) = &mut g.frames {
        let mut shown = [r[0] + 1, r[1] + 1];
        ui.horizontal(|ui| {
            ui.label("Frames (1-based)");
            for v in &mut shown {
                ui.add(egui::DragValue::new(v).range(1..=drawer_core::animation::MAX_FRAMES));
            }
        });
        *r = shown.map(|v| v - 1);
    }
    ui.label("Linked rulers follow explicit layer movements and layer visibility. Painting that expands layer bounds does not move a ruler.");
}
fn geometry_controls(
    ui: &mut egui::Ui,
    kind: &mut Kind,
    t: Affine,
    curve: &mut usize,
    vanishing: &mut u64,
    anchor: &mut Point,
    error: &mut Option<String>,
) {
    match kind {
        Kind::Straight { a, b } | Kind::Mirror { a, b } => {
            point_control(ui, a, t);
            point_control(ui, b, t);
        }
        Kind::Radial { center, arms } => {
            point_control(ui, center, t);
            ui.add(egui::Slider::new(arms, 2..=16).text("Symmetry arms"));
        }
        Kind::Perspective { points } => {
            for p in points {
                point_control(ui, p, t);
            }
        }
        Kind::Special(spec) => match spec {
            Spec::ParallelLine { a, b } | Spec::Guide { a, b } => {
                point_control(ui, a, t);
                point_control(ui, b, t);
            }
            Spec::ParallelCurve { points } => curve_controls(ui, points, t),
            Spec::MultipleCurve { curves } => {
                *curve = (*curve).min(curves.len().saturating_sub(1));
                ui.add(
                    egui::Slider::new(curve, 0..=curves.len().saturating_sub(1))
                        .text("Curve index"),
                );
                if let Some(points) = curves.get_mut(*curve) {
                    curve_controls(ui, points, t);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(curves.len() < 8, egui::Button::new("Add curve"))
                        .clicked()
                    {
                        let p = curves
                            .get(*curve)
                            .cloned()
                            .unwrap_or_else(|| vec![[0.0; 2], [32.0, 0.0]]);
                        curves.push(p.iter().map(|p| [p[0], p[1] + 8.0]).collect());
                        *curve = curves.len() - 1;
                    }
                    if ui
                        .add_enabled(curves.len() > 1, egui::Button::new("Delete curve"))
                        .clicked()
                    {
                        curves.remove(*curve);
                        *curve = 0;
                    }
                });
            }
            Spec::RadialLine { center, guide } => {
                ui.label("Center / base guide");
                point_control(ui, center, t);
                point_control(ui, guide, t);
            }
            Spec::RadialCurve { center, points } => {
                ui.label("Radial center");
                point_control(ui, center, t);
                curve_controls(ui, points, t);
            }
            Spec::ConcentricCircle { center, radius } => {
                point_control(ui, center, t);
                ui.add(
                    egui::Slider::new(radius, 0.25..=1e6)
                        .logarithmic(true)
                        .text("Base radius (page pixels)"),
                );
            }
            Spec::Division {
                a,
                b,
                marks,
                snap_ticks,
            } => {
                point_control(ui, a, t);
                point_control(ui, b, t);
                let mut golden = matches!(marks, Marks::Golden(_));
                let old = golden;
                ui.checkbox(&mut golden, "Golden ratio marks");
                if old != golden {
                    *marks = if golden {
                        Marks::Golden(2)
                    } else {
                        Marks::Equal(4)
                    };
                }
                match marks {
                    Marks::Equal(n) => {
                        ui.add(egui::Slider::new(n, 1..=512).text("Equal divisions"));
                    }
                    Marks::Golden(n) => {
                        ui.add(egui::Slider::new(n, 1..=32).text("Golden levels"));
                    }
                }
                ui.checkbox(
                    snap_ticks,
                    "Snap to tick points within screen snap distance",
                );
            }
            Spec::Perspective(p) => perspective_controls(ui, p, t, vanishing, anchor, error),
        },
    }
}
fn perspective_controls(
    ui: &mut egui::Ui,
    p: &mut Perspective,
    t: Affine,
    selected: &mut u64,
    anchor: &mut Point,
    error: &mut Option<String>,
) {
    let mut fish = p.fisheye;
    if ui
        .checkbox(&mut fish, "Fisheye lens (one/two finite poles)")
        .changed()
    {
        let mut next = p.clone();
        next.fisheye = fish;
        if next.valid() {
            *p = next;
        } else {
            *error = Some("Fisheye needs at most two finite vanishing points".into());
        }
    }
    ui.label("Projection center");
    point_control(ui, &mut p.center, t);
    ui.add(
        egui::Slider::new(&mut p.radius, 0.25..=1e6)
            .logarithmic(true)
            .text("Lens/base radius (page pixels)"),
    );
    ui.add_enabled_ui(!p.horizon_fixed, |ui| {
        ui.add(
            egui::DragValue::new(&mut p.horizon)
                .range(-1e6..=1e6)
                .prefix("Eye height (page Y): "),
        );
    });
    let mut locked = p.horizon_fixed;
    if ui.checkbox(&mut locked, "Fix eye height").changed()
        && let Err(err) = p.lock_horizon(locked)
    {
        *error = Some(err);
    }
    for v in p.points.clone() {
        ui.push_id(("vp", v.id), |ui| {
            ui.label(format!("Vanishing point {}", v.id));
            let mut fixed = v.fixed;
            if ui.checkbox(&mut fixed, "Fix vanishing point").changed() {
                p.points.iter_mut().find(|q| q.id == v.id).unwrap().fixed = fixed;
            }
            let mut value = v.point;
            let mut changed = false;
            ui.add_enabled_ui(!v.fixed, |ui| {
                let map = if v.infinite {
                    Affine([t.0[0], t.0[1], t.0[2], t.0[3], 0.0, 0.0])
                } else {
                    t
                };
                changed = point_control(ui, &mut value, map);
            });
            if changed && let Err(err) = p.move_point(v.id, value) {
                *error = Some(err);
            }
            let mut infinite = v.infinite;
            if ui
                .add_enabled(
                    !v.fixed && !p.fisheye,
                    egui::Checkbox::new(&mut infinite, "Infinitize (parallel direction)"),
                )
                .changed()
                && let Err(err) = p.infinitize(v.id, infinite)
            {
                *error = Some(err);
            }
            if ui
                .add_enabled(
                    p.points.len() > 1,
                    egui::Button::new("Delete vanishing point"),
                )
                .clicked()
                && let Err(err) = p.remove_point(v.id)
            {
                *error = Some(err);
            }
        });
    }
    if ui
        .add_enabled(
            p.points.len() < if p.fisheye { 2 } else { 16 },
            egui::Button::new("Add vanishing point"),
        )
        .clicked()
        && let Err(err) = p.add_point([p.center[0] + p.radius, p.horizon])
    {
        *error = Some(err);
    }
    if !p.points.iter().any(|v| v.id == *selected) {
        *selected = p.points.first().map_or(0, |v| v.id);
    }
    egui::ComboBox::from_id_salt("guide-linked-vp")
        .selected_text(format!("Linked VP {selected}"))
        .show_ui(ui, |ui| {
            for v in &p.points {
                ui.selectable_value(selected, v.id, format!("VP {}", v.id));
            }
        });
    ui.label("New guide anchor");
    point_control(ui, anchor, t);
    if ui
        .add_enabled(p.guides.len() < 64, egui::Button::new("Add linked guide"))
        .clicked()
        && let Err(err) = p.add_guide(*selected, *anchor)
    {
        *error = Some(err);
    }
    for g in p.guides.clone() {
        ui.push_id(("linked-guide", g.id), |ui| {
            ui.label(format!("Guide {} → VP {}", g.id, g.vanishing));
            let mut a = g.anchor;
            if point_control(ui, &mut a, t) {
                p.guides.iter_mut().find(|q| q.id == g.id).unwrap().anchor = a;
            }
            if ui.button("Delete guide").clicked() {
                p.remove_guide(g.id);
            }
        });
    }
}
fn mini_preview(ui: &mut egui::Ui, d: &Document, lines: &[[Point; 2]]) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(260.0, 140.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(35));
    let scale = (252.0 / d.width as f64).min(132.0 / d.height as f64);
    let origin = rect.min + egui::vec2(4.0, 4.0);
    for line in lines {
        let q = line.map(|p| {
            [
                origin.x as f64 + p[0] * scale,
                origin.y as f64 + p[1] * scale,
            ]
        });
        if let Some(q) = drawer_core::aids::clip_line(
            q,
            [
                [rect.min.x as f64, rect.min.y as f64],
                [rect.max.x as f64, rect.max.y as f64],
            ],
        ) {
            painter.line_segment(
                q.map(|p| egui::pos2(p[0] as f32, p[1] as f32)),
                egui::Stroke::new(1.0, egui::Color32::LIGHT_GREEN),
            );
        }
    }
}

pub(crate) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::special_rulers_verify::verify(root)?;
    for pen in [false, true] {
        for layer in [0usize, 1] {
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
            let layer_copy = app.documents[0].layers[0].clone();
            app.documents[0].layers.push(layer_copy);
            app.set_tool(BrushTip::PixelPen);
            app.choose_color([200, 40, 20, 255]);
            app.panels.settings.brush_size = 1.0;
            app.panels.targets.insert(
                app.documents[0].id,
                drawer_core::thumbnail::Target::Raster(layer),
            );
            app.workspaces.store.current.pressure.tools[6].size = false;
            app.workspaces.store.current.pressure.tools[6].opacity = false;
            app.workspaces.store.current.input.profiles[6]
                .pixel
                .pressure = false;
            app.workspaces.store.current.input.profiles[6].minimum_size = 1.0;
            app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.0;
            app.workspaces.store.current.input.smoothing[6] = 0.5;
            app.documents[0].scene.rulers.add(
                "Layer only",
                Kind::Special(Spec::Guide {
                    a: [0.0, 8.5],
                    b: [64.0, 8.5],
                }),
            )?;
            let g = &mut app.documents[0].scene.rulers.guides[0];
            g.scope = Scope::Layer(Target::Raster(0));
            g.display = Display::AppliedLayers;
            app.documents[0].view.zoom = 2.0;
            app.documents[0].view.angle = 90.0;
            app.aids.preferences.guides = true;
            let mut time = 0.0;
            let step = |app: &mut Drawer, time: &mut f64| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, vec![]);
            };
            step(&mut app, &mut time);
            step(&mut app, &mut time);
            if (app.canvas_view.ruler_segments > 0) != (layer == 0) {
                return Err("Canvas ruler display scope did not follow active layer".into());
            }
            let before = app.documents[0].snapshot();
            for (phase, p) in [
                (drawer_core::pointer_input::Phase::Down, [8.5, 30.5]),
                (drawer_core::pointer_input::Phase::Up, [48.5, 30.5]),
            ] {
                let mut e = super::precision::pointer(&ctx, &app, p, phase);
                if pen {
                    e.source = drawer_core::pointer_input::Source {
                        kind: drawer_core::pointer_input::Kind::Pen,
                        device: 114,
                        contact: 1,
                    };
                    e.pressure = Some(0.5);
                }
                app.drawing.virtual_events(vec![e], pen);
                step(&mut app, &mut time);
            }
            let y = if layer == 0 { 8 } else { 30 };
            let other = if layer == 0 { 30 } else { 8 };
            if app.documents[0].layers[layer].pixel(64, 64, 20, y) != [200, 40, 20, 255]
                || app.documents[0].layers[layer].pixel(64, 64, 20, other)[3] != 0
                || app.documents[0].history_counts().0 != 1
            {
                return Err(format!(
                    "Ruler layer/replay input {pen}/{layer}: {:?}",
                    app.status
                ));
            }
            if !app.documents[0].undo() || app.documents[0].snapshot() != before {
                return Err("Scoped ruler stroke single Undo".into());
            }
        }
    }
    for vector in [false, true] {
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
            let id = app.documents[0].scene.lines.add_layer("Vector target")?;
            app.panels.targets.insert(
                app.documents[0].id,
                drawer_core::thumbnail::Target::Vector(id),
            );
        }
        app.panels.settings.brush_size = 2.0;
        app.choose_color([200, 40, 20, 255]);
        app.documents[0].view.angle = 90.0;
        app.documents[0].view.zoom = 2.0;
        drawer_core::files::save(
            &mut app.documents[0],
            &root.join(format!("special-base-{vector}.drawer")),
        )
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
        app.execute_command(&ctx, "special-ruler-tools");
        let owner = target(&app, &app.documents[0]);
        let e = app
            .special_rulers
            .draft
            .as_mut()
            .ok_or("Special ruler command missing")?;
        let id = e.data.add(
            "Saved circle",
            Kind::Special(Spec::ConcentricCircle {
                center: [32.0, 32.0],
                radius: 8.0,
            }),
        )?;
        e.data.guides[0].link = Some(owner);
        e.select(id);
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != original || app.documents[0].dirty {
            return Err("Ruler draft changed saved page".into());
        }
        let r = app
            .special_rulers
            .apply
            .ok_or("Special ruler Apply missing")?;
        click(&mut app, &mut time, r);
        if app.special_rulers.draft.is_some()
            || app.documents[0].scene.rulers.guides.len() != 1
            || app.documents[0].history_counts().0 != 1
        {
            return Err(format!(
                "Special ruler production Apply: {:?}, old button {r:?}, current {:?}, history {:?}",
                app.status,
                app.special_rulers.apply,
                app.documents[0].history_counts()
            ));
        }
        let ruler_state = app.documents[0].snapshot();
        app.execute_command(&ctx, "ruler-follow");
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let r = app
            .special_rulers
            .follow_apply
            .ok_or("Follow ruler Apply missing")?;
        click(&mut app, &mut time, r);
        if app.special_rulers.follow.is_some()
            || app.documents[0].history_counts().0 != 2
            || app.documents[0].scene.rulers != ruler_state.scene.rulers
        {
            return Err(format!("Follow ruler production Apply: {:?}", app.status));
        }
        if vector {
            if app.documents[0].scene.lines.layers[0].paths.len() != 1
                || !app.documents[0].scene.lines.layers[0].paths[0].closed
            {
                return Err("Follow ruler vector path missing".into());
            }
        } else if app.documents[0].layers[0].pixel(64, 64, 40, 32)[3] == 0
            || app.documents[0].layers[0].pixel(64, 64, 32, 32)[3] != 0
        {
            return Err("Follow ruler raster did not trace circle".into());
        }
        let drawn = app.documents[0].snapshot();
        app.execute_command(&ctx, "ruler-move-layer");
        app.special_rulers
            .movement
            .as_mut()
            .ok_or("Layer move command missing")?
            .delta = [3.0, 4.0];
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let r = app
            .special_rulers
            .move_apply
            .ok_or("Layer/ruler move Apply missing")?;
        click(&mut app, &mut time, r);
        if app.special_rulers.movement.is_some()
            || app.documents[0].scene.rulers.guides[0].offset != [3.0, 4.0]
            || app.documents[0].history_counts().0 != 3
        {
            return Err(format!(
                "Linked layer production movement: {:?}",
                app.status
            ));
        }
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != drawn
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != ruler_state
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err("Ruler edit/stroke/movement did not have separate Undo".into());
        }
        app.documents[0].redo();
        app.documents[0].redo();
        app.documents[0].redo();
        let file = root.join(format!("special-ui-{vector}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != app.documents[0].snapshot()
        {
            return Err("Ruler production native reopen".into());
        }
        app.execute_command(&ctx, "special-ruler-tools");
        app.documents[0].edit_pixel(0, 3, 4, [1, 2, 3, 255]);
        let changed = app.documents[0].snapshot();
        if app.apply_special_rulers().is_ok() || app.documents[0].snapshot() != changed {
            return Err("Stale ruler editor changed page".into());
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
        if app.special_rulers.draft.is_some() {
            return Err("Ruler Escape did not cancel".into());
        }
    }
    vector_input_smoke()?;
    Ok(())
}
fn vector_input_smoke() -> Result<(), String> {
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
    let layer = app.documents[0]
        .scene
        .lines
        .add_layer("Constrained vector")?;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Vector(layer),
    );
    app.set_tool(BrushTip::PixelPen);
    app.panels.settings.brush_size = 1.0;
    app.choose_color([200, 40, 20, 255]);
    app.workspaces.store.current.auto_shape.enabled = true;
    app.workspaces.store.current.auto_shape.hold_ms = 5000;
    app.documents[0].scene.rulers.add(
        "Vector scoped",
        Kind::Special(Spec::Guide {
            a: [0.0, 8.5],
            b: [64.0, 8.5],
        }),
    )?;
    app.documents[0].scene.rulers.guides[0].scope = Scope::Layer(Target::Vector(layer));
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, vec![]);
    };
    step(&mut app, &mut time);
    step(&mut app, &mut time);
    for (phase, p) in [
        (drawer_core::pointer_input::Phase::Down, [8.5, 30.5]),
        (drawer_core::pointer_input::Phase::Up, [48.5, 30.5]),
    ] {
        let e = super::precision::pointer(&ctx, &app, p, phase);
        app.drawing.virtual_events(vec![e], false);
        step(&mut app, &mut time);
    }
    if app.documents[0].scene.lines.layers[0]
        .paths
        .first()
        .is_none_or(|p| p.points.len() < 2 || p.points.iter().any(|p| (p[1] - 8.5).abs() > 1e-6))
    {
        return Err(format!(
            "Vector freehand did not apply its ruler scope: {:?}",
            app.status
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn special_ruler_buttons_scope_input_layer_links_and_independent_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
