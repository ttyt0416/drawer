use super::*;
use drawer_core::{
    deform,
    mesh_deform::{Mesh, Pin},
    scene::{Point, Rect},
};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    pub(super) pad: Option<egui::Rect>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    pub(super) resume: Option<egui::Rect>,
}
pub(super) struct Draft {
    pub(super) id: u64,
    pub(super) revision: u64,
    pub(super) frame: u32,
    pub(super) layers: Vec<usize>,
    pub(super) selection: bool,
    pub(super) mesh: Mesh,
    pub(super) puppet: bool,
    pub(super) pins: Vec<Pin>,
    pub(super) selected: usize,
    pub(super) dragging: bool,
    pub(super) chosen: Vec<usize>,
    pub(super) resumed: bool,
    drag_start: Point,
    drag_mesh: Mesh,
    pub(super) base: egui::TextureHandle,
    pub(super) images: Option<[egui::TextureHandle; 2]>,
    pub(super) prepared: Option<drawer_core::sizing::DocumentState>,
    pub(super) quality: crate::transform_review::Quality,
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::mesh_deform_verify::verify(root)?;
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
        app.wait_transform_preview(&ctx, time)
            .expect("Transform preview timeout");
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
    let drag = |app: &mut Drawer, time: &mut f64, pad: egui::Rect| {
        let start = pad.center();
        frame(app, time, pointer(start, true));
        let end = start + egui::vec2(20., 8.);
        frame(app, time, vec![egui::Event::PointerMoved(end)]);
        frame(app, time, pointer(end, false));
    };
    app.execute_command(&ctx, "mesh-puppet");
    if app.mesh_deform.draft.is_none() {
        return Err(format!("Mesh command failed: {:?}", app.status));
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let pad = app.mesh_deform.pad.ok_or("Mesh pad missing")?;
    drag(&mut app, &mut time, pad);
    if app
        .mesh_deform
        .draft
        .as_ref()
        .is_none_or(|e| e.mesh.identity() || e.prepared.is_none())
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err(format!(
            "Mesh drag/automatic preview isolation failed: {:?}",
            app.status
        ));
    }
    let pending = root.join("unconfirmed-mesh.drawer");
    if app.save_to(0, &pending).is_ok() || pending.exists() {
        return Err("Unconfirmed mesh silently saved".into());
    }
    app.cancel_transform_save();
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.mesh_deform.apply.ok_or("Mesh Apply missing")?;
    click(&mut app, &mut time, r);
    if app.mesh_deform.draft.is_some()
        || app.documents[0].snapshot() == original
        || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!(
            "Mesh mouse Apply/one Undo failed: {:?}",
            app.status
        ));
    }
    let native = root.join("mesh-app.drawer");
    app.save_to(0, &native)?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Mesh app native/Undo failed".into());
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    app.execute_command(&ctx, "mesh-puppet");
    app.mesh_deform.draft.as_mut().unwrap().puppet = true;
    app.preview_mesh_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    app.apply_mesh_deform()?;
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Zero-pin puppet destroyed redo/dirty/original".into());
    }
    app.execute_command(&ctx, "mesh-puppet");
    app.mesh_deform.draft.as_mut().unwrap().puppet = true;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let pad = app.mesh_deform.pad.ok_or("Puppet pad missing")?;
    drag(&mut app, &mut time, pad);
    let e = app.mesh_deform.draft.as_ref().unwrap();
    if e.pins.len() != 1 || e.pins[0].vertex != 4 || e.prepared.is_none() {
        return Err(format!(
            "Puppet pin mouse drag failed: {:?}; pins={:?}, selected={}, prepared={}, pad={:?}, currentpad={:?}",
            app.status,
            e.pins,
            e.selected,
            e.prepared.is_some(),
            pad,
            app.mesh_deform.pad
        ));
    }
    {
        let e = app.mesh_deform.draft.as_mut().unwrap();
        let area = Rect {
            min: [0.; 2],
            max: [16.; 2],
        };
        let before = e.geometry(area)?.points[0];
        e.selected = 0;
        e.pin(area)?;
        if e.geometry(area)?.points[0] != before {
            return Err("Adding an anchor jumped away from the existing pose".into());
        }
        e.pins.pop();
        e.selected = 4;
        e.pins[0].degrees = 90.;
        e.pins[0].target = [0.5; 2];
    }
    app.preview_mesh_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    if app.documents[0].snapshot() != original {
        return Err("Unconfirmed puppet rotation changed source".into());
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
    if app.mesh_deform.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Puppet Escape committed draft".into());
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
    app.execute_command(&ctx, "mesh-puppet");
    app.preview_mesh_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    app.mesh_deform.draft.as_mut().unwrap().mesh.points[4] = [2., 2.];
    if app.preview_mesh_deform(&ctx).is_ok()
        || app.mesh_deform.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_mesh_deform().is_ok()
    {
        return Err("Folded cage retained previous prepared result".into());
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    app.mesh_deform.draft = None;
    app.execute_command(&ctx, "mesh-puppet");
    app.preview_mesh_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    app.documents[0].current_frame += 1;
    if app.apply_mesh_deform().is_ok() {
        return Err("Stale mesh frame applied".into());
    }
    app.documents[0].current_frame -= 1;
    Ok(())
}
impl Draft {
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
    fn invalidate(&mut self) {
        self.prepared = None;
        self.images = None;
    }
    pub(super) fn settings(&self) -> drawer_core::mesh_edit::Settings {
        drawer_core::mesh_edit::Settings {
            mesh: self.mesh.clone(),
            puppet: self.puppet,
            pins: self.pins.clone(),
            selected: self.chosen.clone(),
        }
    }
    pub(super) fn area(&self, d: &Document) -> Result<Rect, String> {
        if self.resumed {
            return d
                .scene
                .mesh_edit
                .as_ref()
                .map(|e| e.area)
                .ok_or("Saved mesh edit disappeared".into());
        }
        Ok(
            deform::area(d, &self.layers, self.selection)?.unwrap_or(Rect {
                min: [0.; 2],
                max: [d.width as f64, d.height as f64],
            }),
        )
    }
    pub(super) fn geometry(&self, area: Rect) -> Result<Mesh, String> {
        self.settings().geometry(area)
    }
    fn pin(&mut self, area: Rect) -> Result<(), String> {
        if self.pins.iter().any(|p| p.vertex == self.selected) {
            return Err("A pin already occupies this vertex".into());
        }
        let pose = self.geometry(area)?;
        self.pins.push(Pin {
            vertex: self.selected,
            target: pose.points[self.selected],
            degrees: 0.,
            locked: false,
            ..Default::default()
        });
        self.invalidate();
        Ok(())
    }
}
impl Drawer {
    pub(super) fn begin_mesh_deform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.finish_stroke();
        self.transform_begin_guard(2)?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let layer = match self.panels.targets.get(&d.id) {
            None => 0,
            Some(drawer_core::thumbnail::Target::Raster(n)) => *n,
            _ => return Err("Select a raster layer".into()),
        };
        let layers = deform::targets(d, &[layer])?;
        self.mesh_deform.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layers,
            quality: Default::default(),
            selection: false,
            mesh: Mesh::new(3, 3)?,
            puppet: false,
            pins: vec![],
            selected: 4,
            dragging: false,
            chosen: vec![4],
            resumed: false,
            drag_start: [0.; 2],
            drag_mesh: Mesh::new(3, 3)?,
            base: crate::layer_merge::image(ctx, d, "mesh-source"),
            images: None,
            prepared: None,
        }));
        Ok(())
    }
    pub(super) fn resume_mesh_deform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let saved = d.scene.mesh_edit.as_ref().ok_or("No saved mesh edit")?;
        let original = saved.original(d)?;
        let e = self
            .mesh_deform
            .draft
            .as_mut()
            .ok_or("Open mesh deformation first")?;
        e.layers = saved.layers();
        e.selection = saved.selection;
        e.mesh = saved.settings.mesh.clone();
        e.puppet = saved.settings.puppet;
        e.pins = saved.settings.pins.clone();
        e.chosen = saved.settings.selected.clone();
        e.selected = e.chosen.first().copied().unwrap_or(0);
        e.resumed = true;
        e.dragging = false;
        e.base = crate::layer_merge::image(ctx, &original, "saved-mesh-source");
        e.invalidate();
        Ok(())
    }
    pub(super) fn preview_mesh_deform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.mesh_deform.draft {
            e.prepared = None;
            e.images = None;
        }
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .mesh_deform
            .draft
            .as_ref()
            .ok_or("Transform window closed")?;
        if !((d.id, d.revision, d.current_frame) == (e.id, e.revision, e.frame)) {
            return Err("Page/frame/target changed; reopen the transform".into());
        }
        self.start_transform_preview(ctx, 2)
    }
    pub(super) fn apply_mesh_deform(&mut self) -> Result<(), String> {
        self.transform_apply_guard(2)?;
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .mesh_deform
            .draft
            .as_mut()
            .ok_or("Mesh window closed")?;
        if !e.fresh(&self.documents[n]) {
            return Err("Page/frame/revision changed; reopen mesh deformation".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Prepare a valid mesh preview first")?;
        self.documents[n].commit_state(state)?;
        self.mesh_deform.draft = None;
        self.status = None;
        Ok(())
    }
    pub(super) fn mesh_deform_window(&mut self, ctx: &egui::Context) {
        let working = self.transform_working();
        let Some(mut e) = self.mesh_deform.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        let mut cancel = false;
        let mut error = None;
        let mut resume = false;
        egui::Window::new("Mesh and puppet deformation").default_width(560.).default_pos(egui::pos2(25.,40.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame changed; close and reopen.");}
            ui.small("Apply or cancel this unconfirmed deformation before saving. Escape/close discards the cage and all pins.");
            let saved=d.scene.mesh_edit.as_ref();
            let r=ui.add_enabled(saved.is_some(),egui::Button::new("Resume saved mesh / pins from original pixels"));self.mesh_deform.resume=Some(r.rect);resume=r.clicked();
            if e.resumed{ui.small("Editing the saved original pixels. Source layers and selection stay fixed; unrelated layer properties are retained.");}
            let mut changed=false;
            changed|=crate::transform_review::quality(ui,&mut e.quality);
            if working {ui.label("Calculating transform — close or Escape to cancel");}ui.horizontal(|ui|{changed|=ui.selectable_value(&mut e.puppet,false,"Mesh grid").changed();changed|=ui.selectable_value(&mut e.puppet,true,"Puppet pins").changed();});
            ui.collapsing("Raster layers",|ui|{egui::ScrollArea::vertical().max_height(65.).show(ui,|ui|{for(n,l)in d.layers.iter().enumerate(){let mut on=e.layers.contains(&n);if ui.add_enabled(!e.resumed,egui::Checkbox::new(&mut on,&l.name)).changed(){if on{e.layers.push(n);}else{e.layers.retain(|k|*k!=n);}changed=true;}}});});
            changed|=ui.add_enabled(!e.resumed,egui::Checkbox::new(&mut e.selection,"Only selected pixels")).changed();
            ui.horizontal(|ui|{let mut c=e.mesh.columns;let mut r=e.mesh.rows;let resized=ui.add(egui::DragValue::new(&mut c).range(2..=5).prefix("Columns ")).changed()|ui.add(egui::DragValue::new(&mut r).range(2..=5).prefix("Rows ")).changed();if resized{e.mesh=Mesh::new(c,r).unwrap();e.pins.clear();e.selected=0;e.chosen=vec![0];changed=true;}
if ui.button("Reset cage and pins").clicked(){e.mesh=Mesh::new(c,r).unwrap();e.pins.clear();e.chosen=vec![e.selected.min(e.mesh.points.len()-1)];changed=true;}});
            let area=e.area(d);
            ui.horizontal(|ui|{
                ui.label("Vertex");if ui.add(egui::DragValue::new(&mut e.selected).range(0..=e.mesh.points.len()-1)).changed(){e.chosen=vec![e.selected];changed=true;}
                if e.puppet{if ui.button("Add pin").clicked()&&let Err(err)=area.as_ref().map_err(Clone::clone).and_then(|a|e.pin(*a)){error=Some(err);}
                    if ui.button("Remove pin").clicked(){e.pins.retain(|p|p.vertex!=e.selected);changed=true;}
                }else{for v in &mut e.mesh.points[e.selected]{changed|=ui.add(egui::DragValue::new(v).range(-64. ..=64.).speed(0.01)).changed();}}
            });
            if !e.puppet{ui.horizontal(|ui|{ui.label(format!("{} selected points",e.chosen.len()));if ui.button("Select all points").clicked(){e.chosen=(0..e.mesh.points.len()).collect();changed=true;}
if ui.button("Clear point selection").clicked(){e.chosen.clear();changed=true;}});
                ui.horizontal(|ui|{for (axis,label)in [(0,"Move selected X "),(1,"Move selected Y ")]{let mut value=0.;if ui.add(egui::DragValue::new(&mut value).speed(0.01).prefix(label)).changed(){let mut delta=[0.;2];delta[axis]=value;if let Err(err)=e.mesh.move_selected(&e.chosen,delta){error=Some(err);}else{changed=true;}}}});
            }
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),85.),egui::Layout::top_down(egui::Align::Min),|ui|{
            if e.puppet{if let Some(pin)=e.pins.iter_mut().find(|p|p.vertex==e.selected){
                changed|=ui.checkbox(&mut pin.locked,"Fixed pin (keep its original position)").changed();
                ui.horizontal(|ui|{changed|=ui.add(egui::DragValue::new(&mut pin.depth).prefix("Depth ")).changed();changed|=ui.checkbox(&mut pin.keep_angle,"Keep reference angle").changed();if ui.button("Capture reference angle").clicked(){pin.reference_degrees=pin.degrees;changed=true;}});
                ui.add_enabled_ui(!pin.locked,|ui|{ui.horizontal(|ui|{for v in &mut pin.target{changed|=ui.add(egui::DragValue::new(v).range(-64. ..=64.).speed(0.01).prefix("Target ")).changed();}changed|=ui.add(egui::DragValue::new(&mut pin.degrees).range(-36000. ..=36000.).speed(1.).suffix("°")).changed();});});
            }else{ui.small("Choose a vertex and Add pin, or drag a vertex to create a movable pin. Other fixed pins anchor the cage.");}}
            });
            if changed{e.invalidate();}
            let geometry=area.as_ref().map_err(Clone::clone).and_then(|a|e.geometry(*a));let valid=geometry.as_ref().is_ok_and(Mesh::valid);
            if !valid{ui.colored_label(egui::Color32::RED,"Collapsed triangle or invalid cage/pin: preview cannot be applied. Puppet overlaps use depth, then creation order.");}
            ui.horizontal(|ui|{
                let(r,_)=ui.allocate_exact_size(egui::vec2(280.,210.),egui::Sense::hover());let response=ui.interact(r,ui.make_persistent_id("mesh-deformation-pad"),egui::Sense::click_and_drag());self.mesh_deform.pad=Some(r);
                let image=e.images.as_ref().map_or(&e.base,|i|&i[1]);ui.painter().image(image.id(),r,egui::Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),egui::Color32::WHITE);
                if let(Ok(a),Ok(mesh))=(&area,&geometry){
                    let to_screen=|p:Point|egui::pos2(r.min.x+((a.min[0]+p[0]*(a.max[0]-a.min[0]))/d.width as f64)as f32*r.width(),r.min.y+((a.min[1]+p[1]*(a.max[1]-a.min[1]))/d.height as f64)as f32*r.height());
                    let color=if valid{egui::Color32::WHITE}else{egui::Color32::RED};
                    for triangle in mesh.triangles(){for k in 0..3{ui.painter().line_segment([to_screen(mesh.points[triangle[k]]),to_screen(mesh.points[triangle[(k+1)%3]])],egui::Stroke::new(1.,color));}}
                    for(n,p)in mesh.points.iter().enumerate(){let pinned=e.pins.iter().any(|p|p.vertex==n);ui.painter().circle_filled(to_screen(*p),if e.chosen.contains(&n){6.}else{4.},if pinned{egui::Color32::GOLD}else{color});}
                    if fresh&&(response.clicked()||response.drag_started())&&let Some(pos)=response.interact_pointer_pos(){
                        let origin=if response.drag_started(){ctx.input(|i|i.pointer.press_origin()).unwrap_or(pos)}else{pos};
let nearest=mesh.points.iter().enumerate().min_by(|(_,a),(_,b)|to_screen(**a).distance_sq(origin).total_cmp(&to_screen(**b).distance_sq(origin)));
                        if let Some((n,p))=nearest&&to_screen(*p).distance(origin)<=14.{
                            let shift=ctx.input(|i|i.modifiers.shift);
                            if shift&&response.clicked(){if e.chosen.contains(&n){e.chosen.retain(|v|*v!=n);}else{e.chosen.push(n);}}
                            else if !e.chosen.contains(&n){if !shift{e.chosen.clear();}e.chosen.push(n);}
                            e.selected=n;e.dragging=response.drag_started();
                            e.drag_start=[(origin.x-r.min.x)as f64/r.width()as f64*d.width as f64,(origin.y-r.min.y)as f64/r.height()as f64*d.height as f64];
                            e.drag_start=[0,1].map(|k|(e.drag_start[k]-a.min[k])/(a.max[k]-a.min[k]));
                            e.drag_mesh=mesh.clone();e.invalidate();
                            if e.dragging&&e.puppet&&!e.pins.iter().any(|p|p.vertex==n)&&let Err(err)=e.pin(*a){error=Some(err);}
                        }
                    }
                    if fresh&&response.dragged()&&e.dragging&&let Some(pos)=response.interact_pointer_pos(){
                        let point=[(pos.x-r.min.x)as f64/r.width()as f64*d.width as f64,(pos.y-r.min.y)as f64/r.height()as f64*d.height as f64];
                        let delta=[0,1].map(|k|(point[k]-a.min[k])/(a.max[k]-a.min[k])-e.drag_start[k]);
                        if e.puppet{if let Some(pin)=e.pins.iter_mut().find(|p|p.vertex==e.selected){if pin.locked{error=Some("This pin is fixed; unlock it to move".into());}else{pin.target=[0,1].map(|k|e.drag_mesh.points[e.selected][k]+delta[k]);e.invalidate();}}}
                        else{let mut next=e.drag_mesh.clone();if let Err(err)=next.move_selected(&e.chosen,delta){error=Some(err);}else{e.mesh=next;e.invalidate();}}
                    }
                    if response.drag_stopped(){preview=e.dragging;e.dragging=false;}
                }
                ui.vertical(|ui|{ui.label("Shift-click points to extend selection; drag a selected point to move the set");ui.small("White: grid / gold: pin. Coordinates are normalized editing bounds. A pin rotation uses document pixels, preserving aspect ratio.");if let Some(images)=&e.images{ui.label("Original");ui.add(egui::Image::new(&images[0]).fit_to_exact_size(egui::vec2(100.,75.)));}});
            });
            if let Err(err)=geometry{ui.colored_label(egui::Color32::YELLOW,err);}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare mesh / puppet preview"));self.mesh_deform.preview=Some(r.rect);preview|=r.clicked();cancel=ui.button("Cancel transform").clicked();
            let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply mesh / puppet as one Undo"));self.mesh_deform.apply=Some(r.rect);apply=r.clicked();
        });
        if open && !cancel {
            self.mesh_deform.draft = Some(e);
        }
        if resume && let Err(err) = self.resume_mesh_deform(ctx) {
            error = Some(err);
        }
        if preview && let Err(err) = self.preview_mesh_deform(ctx) {
            error = Some(err);
        }
        if apply && let Err(err) = self.apply_mesh_deform() {
            error = Some(err);
        }
        if let Some(err) = error {
            self.status = Some(err);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mesh_puppet_mouse_preview_apply_zero_pins_cancel_fold_stale_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
