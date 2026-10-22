use super::*;
use drawer_core::{
    deform as core,
    material_mapping::{UNIT, Warp},
    scene::Point,
};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    pad: Option<egui::Rect>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layers: Vec<usize>,
    selection: bool,
    kind: u8,
    quad: [Point; 4],
    mode: core::Mode,
    radius: f64,
    strength: f64,
    only_refer: bool,
    dabs: Vec<core::Dab>,
    strokes: Vec<usize>,
    last: Option<Point>,
    corner: Option<usize>,
    base: egui::TextureHandle,
    prepared: Option<drawer_core::sizing::DocumentState>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Draft {
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
    fn invalidate(&mut self) {
        self.prepared = None;
        self.images = None;
    }
    fn dab(&mut self, point: Point, delta: Point) -> Result<(), String> {
        if self.dabs.len() >= 512 {
            return Err("Liquify has512 dabs; apply or undo a stroke before continuing".into());
        }
        self.dabs.push(core::Dab {
            mode: self.mode,
            center: point,
            delta,
            radius: self.radius,
            strength: self.strength,
        });
        self.invalidate();
        Ok(())
    }
}
impl Drawer {
    pub(super) fn deformation_save_guard(&self, index: usize) -> Result<(), String> {
        if self
            .deform
            .draft
            .as_ref()
            .is_some_and(|e| e.id == self.documents[index].id)
        {
            Err("Unconfirmed deformation: apply it, or cancel the deformation window, before saving.".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn begin_deform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let layer = match self.panels.targets.get(&d.id) {
            None => 0,
            Some(drawer_core::thumbnail::Target::Raster(n)) => *n,
            _ => {
                return Err(
                    "Select a raster layer; convert other source types before deforming".into(),
                );
            }
        };
        let layers = core::targets(d, &[layer])?;
        self.deform.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layers,
            selection: false,
            kind: 0,
            quad: UNIT,
            mode: core::Mode::Push,
            radius: 32.,
            strength: 0.5,
            only_refer: false,
            dabs: vec![],
            strokes: vec![],
            last: None,
            corner: None,
            base: crate::layer_merge::image(ctx, d, "deform-source"),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_deform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .deform
            .draft
            .as_mut()
            .ok_or("Deformation window closed")?;
        e.invalidate();
        if !e.fresh(d) {
            return Err("Page/frame/revision changed; reopen deformation".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(5))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Deformation preview workspace limit".into());
        }
        let warp = if e.kind == 0 {
            Warp::Distort(e.quad)
        } else {
            Warp::Perspective(e.quad)
        };
        let change = if e.kind == 2 {
            core::Change::Liquify {
                dabs: &e.dabs,
                only_refer: e.only_refer,
            }
        } else {
            core::Change::Quad(&warp)
        };
        let state = core::prepare(d, &e.layers, e.selection, change)?;
        if used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Deformation result workspace limit".into());
        }
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "deform-before"),
            crate::layer_merge::image(ctx, &after, "deform-after"),
        ]);
        e.prepared = Some(state);
        self.status = None;
        Ok(())
    }
    fn apply_deform(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .deform
            .draft
            .as_mut()
            .ok_or("Deformation window closed")?;
        if !e.fresh(&self.documents[n]) {
            return Err("Page/frame/revision changed; reopen deformation".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Prepare a valid deformation preview first")?;
        self.documents[n].commit_state(state)?;
        self.deform.draft = None;
        self.status = None;
        Ok(())
    }
    pub(super) fn deform_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.deform.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        let mut error = None;
        egui::Window::new("Free / perspective / liquify").default_width(570.).default_pos(egui::pos2(20.,35.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame changed; close and reopen.");}
            ui.small("Unconfirmed deformation: apply or cancel before saving. Escape/close discards every draft operation.");
            let mut changed=false;
            ui.horizontal(|ui|{for(i,label)in["Four-corner free","Perspective","Liquify"].into_iter().enumerate(){changed|=ui.selectable_value(&mut e.kind,i as u8,label).changed();}});
            ui.collapsing("Raster layers (linked raster members are included)",|ui|{egui::ScrollArea::vertical().max_height(70.).show(ui,|ui|{for(n,l)in d.layers.iter().enumerate(){let mut on=e.layers.contains(&n);if ui.checkbox(&mut on,&l.name).changed(){if on{e.layers.push(n);}else{e.layers.retain(|v|*v!=n);}changed=true;}}});});
            changed|=ui.checkbox(&mut e.selection,"Only selected pixels").changed();
            if e.kind==2{
                ui.horizontal(|ui|{egui::ComboBox::from_id_salt("liquify-mode").selected_text(e.mode.label()).show_ui(ui,|ui|{for m in core::Mode::ALL{ui.selectable_value(&mut e.mode,m,m.label());}});ui.add(egui::DragValue::new(&mut e.radius).range(0.5..=1024.).prefix("Radius "));ui.add(egui::Slider::new(&mut e.strength,0. ..=1.).text("Strength"));});
                changed|=ui.checkbox(&mut e.only_refer,"Only refer to editing area").changed();
                ui.horizontal(|ui|{if ui.button("Undo draft stroke").clicked()&&let Some(n)=e.strokes.pop(){e.dabs.truncate(n);changed=true;}
if ui.button("Clear strokes").clicked(){e.dabs.clear();e.strokes.clear();changed=true;}ui.label(format!("{} /512 dabs",e.dabs.len()));});
            }else{
                ui.horizontal(|ui|{for(n,p)in e.quad.iter_mut().enumerate(){ui.vertical(|ui|{ui.label(format!("Corner {}",n+1));for v in p{changed|=ui.add(egui::DragValue::new(v).speed(0.01).range(-64. ..=64.)).changed();}});}if ui.button("Reset corners").clicked(){e.quad=UNIT;changed=true;}});
                ui.small("Corners use normalized editing bounds (0–1). Drag a white corner below; reversed/folded quads are rejected.");
            }
            if changed{e.invalidate();}
            let area=core::area(d,&e.layers,e.selection);
            ui.horizontal(|ui|{
                let size=egui::vec2(250.,190.);let(r,response)=ui.allocate_exact_size(size,egui::Sense::drag());self.deform.pad=Some(r);
                ui.painter().rect_filled(r,0.,egui::Color32::from_gray(48));
                {let image=e.images.as_ref().map_or(&e.base,|images|&images[1]);ui.painter().image(image.id(),r,egui::Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),egui::Color32::WHITE);}
                let to_doc=|p:egui::Pos2|[(p.x-r.min.x) as f64/r.width() as f64*d.width as f64,(p.y-r.min.y) as f64/r.height() as f64*d.height as f64];
                let to_screen=|p:Point|egui::pos2(r.min.x+(p[0]/d.width as f64) as f32*r.width(),r.min.y+(p[1]/d.height as f64) as f32*r.height());
                if e.kind<2&&let Ok(Some(a))=&area {for (n,p) in e.quad.iter().enumerate(){let point=[0,1].map(|k|a.min[k]+p[k]*(a.max[k]-a.min[k]));let next=e.quad[(n+1)%4];let next=[0,1].map(|k|a.min[k]+next[k]*(a.max[k]-a.min[k]));ui.painter().line_segment([to_screen(point),to_screen(next)],egui::Stroke::new(1.,egui::Color32::WHITE));ui.painter().circle_filled(to_screen(point),5.,egui::Color32::WHITE);}}
                if fresh&&response.drag_started()&&let Some(pos)=response.interact_pointer_pos(){let p=to_doc(pos);e.last=Some(p);if e.kind==2{e.strokes.push(e.dabs.len());if let Err(err)=e.dab(p,[0.;2]){error=Some(err);}}else if let Ok(Some(a))=&area {e.corner=(0..4).min_by(|x,y|{let distance=|n:usize|{let v=to_screen([0,1].map(|k|a.min[k]+e.quad[n][k]*(a.max[k]-a.min[k])));v.distance_sq(pos)};distance(*x).total_cmp(&distance(*y))});}}
                if fresh&&response.dragged()&&let Some(pos)=response.interact_pointer_pos(){let p=to_doc(pos);if e.kind==2{if let Some(last)=e.last {let delta=[p[0]-last[0],p[1]-last[1]];let steps=(delta[0].hypot(delta[1])/(e.radius*0.25).max(0.5)).ceil().clamp(1.,64.) as usize;if delta!=[0.;2]{for n in 1..=steps{let center=[0,1].map(|k|last[k]+delta[k]*n as f64/steps as f64);if let Err(err)=e.dab(center,delta.map(|v|v/steps as f64)){error=Some(err);break;}}}}}else if let (Some(n),Ok(Some(a)))=(e.corner,&area){e.quad[n]=[0,1].map(|k|((p[k]-a.min[k])/(a.max[k]-a.min[k])).clamp(-64.,64.));e.invalidate();}e.last=Some(p);}
                if response.drag_stopped(){e.last=None;e.corner=None;preview=fresh;}
                ui.vertical(|ui|{ui.label("Mouse / pen editing pad");ui.small("The pad maps the page to this rectangle. Drag to deform; a preview is prepared on release.");if let Some(images)=&e.images{ui.label("Original");ui.add(egui::Image::new(&images[0]).fit_to_exact_size(egui::vec2(100.,76.)));}});
            });
            if let Err(err)=area{ui.colored_label(egui::Color32::YELLOW,err);}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare deformation preview"));self.deform.preview=Some(r.rect);preview|=r.clicked();
            let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply deformation as one Undo"));self.deform.apply=Some(r.rect);apply=r.clicked();
        });
        if open {
            self.deform.draft = Some(e);
        }
        if preview && let Err(err) = self.preview_deform(ctx) {
            error = Some(err);
        }
        if apply && let Err(err) = self.apply_deform() {
            error = Some(err);
        }
        if let Some(err) = error {
            self.status = Some(err);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::deform_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::deform_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    let frame = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        crate::input_replay::frame(&ctx, app, *time, events);
    };
    let pointer = |pos: egui::Pos2, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    };
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            frame(app, time, pointer(r.center(), pressed));
        }
    };
    app.execute_command(&ctx, "raster-deformation");
    app.deform
        .draft
        .as_mut()
        .ok_or("Deformation command did not open")?
        .quad = UNIT.map(|p| [p[0] + 0.125, p[1]]);
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app
        .deform
        .preview
        .ok_or("Deformation preview button missing")?;
    click(&mut app, &mut time, r);
    if app
        .deform
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err(format!(
            "Deformation mouse preview/isolation: {:?}",
            app.status
        ));
    }
    let unconfirmed = root.join("unconfirmed-deformation.drawer");
    if app.save_to(0, &unconfirmed).is_ok() || unconfirmed.exists() {
        return Err("Unconfirmed deformation saved without notice".into());
    }
    app.execute_command(&ctx, "save");
    if app
        .status
        .as_ref()
        .is_none_or(|s| !s.contains("Unconfirmed deformation"))
    {
        return Err("Save did not explain pending deformation".into());
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.deform.apply.ok_or("Deformation Apply button missing")?;
    click(&mut app, &mut time, r);
    if app.deform.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].snapshot() == original
    {
        return Err("Deformation mouse Apply/one Undo".into());
    }
    let native = root.join("deformation-app.drawer");
    app.save_to(0, &native)?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Deformation native/Undo failed".into());
    }
    app.execute_command(&ctx, "raster-deformation");
    {
        let e = app.deform.draft.as_mut().unwrap();
        e.kind = 2;
        e.mode = core::Mode::Expand;
        e.radius = 5.;
        e.strength = 1.;
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let pad = app.deform.pad.ok_or("Liquify pad missing")?;
    let start = pad.center();
    frame(&mut app, &mut time, pointer(start, true));
    for dx in [4., 8., 12.] {
        frame(
            &mut app,
            &mut time,
            vec![egui::Event::PointerMoved(start + egui::vec2(dx, 0.))],
        );
    }
    frame(
        &mut app,
        &mut time,
        pointer(start + egui::vec2(12., 0.), false),
    );
    let e = app.deform.draft.as_ref().unwrap();
    if e.dabs.len() < 2 || e.prepared.is_none() || app.documents[0].snapshot() != original {
        return Err(format!(
            "Liquify mouse stroke/automatic preview/isolation: {:?}",
            app.status
        ));
    }
    frame(
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
    if app.deform.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Escape did not discard accumulated liquify".into());
    }
    frame(
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
    app.execute_command(&ctx, "raster-deformation");
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let pad = app.deform.pad.ok_or("Quad pad missing")?;
    let start = pad.min + egui::vec2(2., 2.);
    frame(&mut app, &mut time, pointer(start, true));
    let end = start + egui::vec2(18., 12.);
    frame(&mut app, &mut time, vec![egui::Event::PointerMoved(end)]);
    frame(&mut app, &mut time, pointer(end, false));
    if app
        .deform
        .draft
        .as_ref()
        .is_none_or(|e| e.quad == UNIT || e.prepared.is_none())
        || app.documents[0].snapshot() != original
    {
        return Err(format!("Quad corner mouse drag failed: {:?}", app.status));
    }
    app.preview_deform(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_deform().is_ok() {
        return Err("Stale deformation frame applied".into());
    }
    app.documents[0].current_frame -= 1;
    app.deform.draft.as_mut().unwrap().quad = [[0.; 2]; 4];
    if app.preview_deform(&ctx).is_ok()
        || app.deform.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_deform().is_ok()
    {
        return Err("Failed quad retained earlier prepared state".into());
    }
    app.deform.draft = None;
    app.execute_command(&ctx, "raster-deformation");
    {
        let e = app.deform.draft.as_mut().unwrap();
        e.kind = 2;
        let mut b = core::Dab {
            mode: core::Mode::Push,
            center: [8., 8.],
            delta: [1., 0.],
            radius: 4.,
            strength: 0.,
        };
        e.dabs.push(b);
        b.center = [7., 7.];
        e.dabs.push(b);
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    app.preview_deform(&ctx)?;
    app.apply_deform()?;
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Zero liquify destroyed redo/dirty/source".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn deformation_mouse_preview_apply_save_notice_liquify_cancel_stale_zero() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
