use super::*;
use drawer_core::{
    gradient_ramp::{Ramp, Space, Stop},
    tone_mapping::{self as core, Curve, Operation},
};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    graph: Option<egui::Rect>,
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::tone_mapping_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 256,
        height: 1,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::tone_mapping_verify::fixture()?.snapshot())?;
    let mut time = 0.;
    let frame = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        crate::input_replay::frame(&ctx, app, *time, events);
    };
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            frame(
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
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    app.execute_command(&ctx, "tone-curves");
    let e = app
        .tone_mapping
        .draft
        .as_mut()
        .ok_or("Tone curves command did not open")?;
    if let Operation::Curves(c) = &mut e.op {
        c[0].points = vec![[0., 0.], [0.5, 0.5], [1., 1.]];
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.tone_mapping.graph.ok_or("Curve graph missing")?;
    let from = r.center();
    let to = egui::pos2(from.x, r.bottom() - r.height() * 0.8);
    frame(
        &mut app,
        &mut time,
        vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    frame(&mut app, &mut time, vec![egui::Event::PointerMoved(to)]);
    frame(
        &mut app,
        &mut time,
        vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
    if !matches!(&app.tone_mapping.draft.as_ref().unwrap().op,Operation::Curves(c)if(c[0].points[1][1]-0.8).abs()<1e-5)
        || app.documents[0].snapshot() != original
    {
        return Err("Curve graph mouse drag/source isolation".into());
    }
    let r = app
        .tone_mapping
        .preview
        .ok_or("Tone preview button missing")?;
    click(&mut app, &mut time, r);
    if app
        .tone_mapping
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err(format!(
            "Tone preview failed or changed source: {:?}",
            app.status
        ));
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.tone_mapping.apply.ok_or("Tone Apply button missing")?;
    if !ctx.content_rect().contains(r.center()) {
        return Err(format!("Tone Apply is outside viewport: {r:?}"));
    }
    click(&mut app, &mut time, r);
    if app.tone_mapping.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].snapshot() == original
        || app.documents[0].layers[0]
            .pixels
            .iter()
            .zip(&original.layers[0].pixels)
            .any(|(a, b)| a[3] != b[3])
    {
        return Err("Tone Apply/one Undo/alpha".into());
    }
    let native = root.join("tone-mapping-app.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Tone native/Undo".into());
    }
    app.execute_command(&ctx, "gradient-map");
    if let Operation::Gradient(r) = &mut app.tone_mapping.draft.as_mut().unwrap().op {
        *r = Ramp::two([0, 0, 0, 255], [255, 50, 20, 255]);
    }
    app.preview_tone_mapping(&ctx)?;
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
    if app.tone_mapping.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Gradient map Escape committed preview".into());
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
    app.execute_command(&ctx, "gradient-map");
    app.preview_tone_mapping(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_tone_mapping().is_ok() {
        return Err("Stale tone frame applied".into());
    }
    app.documents[0].current_frame -= 1;
    app.tone_mapping.draft.as_mut().unwrap().op =
        Operation::Curves(std::array::from_fn(|_| Curve {
            points: vec![[0., 0.], [0.5, 0.7], [0.5, 0.2], [1., 1.]],
            smooth: true,
        }));
    if app.preview_tone_mapping(&ctx).is_ok()
        || app.tone_mapping.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_tone_mapping().is_ok()
        || app.documents[0].snapshot() != original
    {
        return Err("Invalid curve retained an applicable prior plan".into());
    }
    Ok(())
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: usize,
    selection: bool,
    op: Operation,
    channel: usize,
    grab: Option<usize>,
    prepared: Option<drawer_core::sizing::DocumentState>,
    images: Option<[egui::TextureHandle; 2]>,
}
fn layer(app: &Drawer, id: u64) -> Option<usize> {
    match app.panels.targets.get(&id) {
        None => Some(0),
        Some(drawer_core::thumbnail::Target::Raster(n)) => Some(*n),
        _ => None,
    }
}
impl Draft {
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
}
fn graph(ui: &mut egui::Ui, c: &mut Curve, grab: &mut Option<usize>) -> egui::Rect {
    let (r, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(360.), 120.),
        egui::Sense::click_and_drag(),
    );
    let map = |p: [f64; 2]| {
        egui::pos2(
            r.left() + p[0] as f32 * r.width(),
            r.bottom() - p[1] as f32 * r.height(),
        )
    };
    ui.painter()
        .rect_filled(r, 0., egui::Color32::from_gray(24));
    for n in 0..=4 {
        let x = n as f32 / 4.;
        ui.painter().line_segment(
            [
                egui::pos2(r.left() + x * r.width(), r.top()),
                egui::pos2(r.left() + x * r.width(), r.bottom()),
            ],
            egui::Stroke::new(1., egui::Color32::from_gray(55)),
        );
        ui.painter().line_segment(
            [
                egui::pos2(r.left(), r.top() + x * r.height()),
                egui::pos2(r.right(), r.top() + x * r.height()),
            ],
            egui::Stroke::new(1., egui::Color32::from_gray(55)),
        );
    }
    if c.valid() {
        let line = (0..=128)
            .map(|n| {
                let x = n as f64 / 128.;
                map([x, c.sample(x).unwrap()])
            })
            .collect::<Vec<_>>();
        ui.painter().add(egui::Shape::line(
            line,
            egui::Stroke::new(2., egui::Color32::LIGHT_BLUE),
        ));
        for p in &c.points {
            ui.painter()
                .circle_filled(map(*p), 4., egui::Color32::WHITE);
        }
        if response.hovered()
            && ui.input(|i| i.pointer.primary_pressed())
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            *grab = c
                .points
                .iter()
                .enumerate()
                .filter(|(_, p)| map(**p).distance(pos) <= 10.)
                .min_by(|(_, a), (_, b)| map(**a).distance(pos).total_cmp(&map(**b).distance(pos)))
                .map(|(i, _)| i);
        }
        if let Some(i) = *grab
            && ui.input(|i| i.pointer.primary_down())
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            let y = ((r.bottom() - pos.y) / r.height()).clamp(0., 1.) as f64;
            let x = if i == 0 {
                0.
            } else if i + 1 == c.points.len() {
                1.
            } else {
                let a = c.points[i - 1][0];
                let b = c.points[i + 1][0];
                let gap = ((b - a) * 1e-6).max(1e-9);
                (((pos.x - r.left()) / r.width()).clamp(0., 1.) as f64).clamp(a + gap, b - gap)
            };
            c.points[i] = [x, y];
        }
    } else {
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            "Separate overlapping input points",
            egui::FontId::proportional(14.),
            egui::Color32::YELLOW,
        );
    }
    if ui.input(|i| i.pointer.any_released()) {
        *grab = None;
    }
    r
}
impl Drawer {
    pub(super) fn begin_tone_mapping(&mut self, map: bool) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let layer = layer(self, d.id).ok_or("Select a raster layer")?;
        self.tone_mapping.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            selection: false,
            op: if map {
                Operation::Gradient(Ramp::two([0, 0, 0, 255], [255; 4]))
            } else {
                Operation::identity_curves()
            },
            channel: 0,
            grab: None,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_tone_mapping(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = layer(self, d.id);
        let e = self
            .tone_mapping
            .draft
            .as_mut()
            .ok_or("Tone mapping window closed")?;
        e.prepared = None;
        e.images = None;
        if !e.fresh(d) || target != Some(e.layer) {
            return Err("Page/frame/target changed; reopen tone mapping".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(5))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Tone mapping preview exceeds workspace memory".into());
        }
        let state = core::prepare(d, e.layer, e.selection, &e.op)?;
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "tone-map-before"),
            crate::layer_merge::image(ctx, &after, "tone-map-after"),
        ]);
        e.prepared = Some(state);
        Ok(())
    }
    fn apply_tone_mapping(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let target = layer(self, self.documents[n].id);
        let e = self
            .tone_mapping
            .draft
            .as_mut()
            .ok_or("Tone mapping window closed")?;
        if !e.fresh(&self.documents[n]) || target != Some(e.layer) {
            return Err("Page/frame/target changed; preview again".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Prepare tone mapping preview first")?;
        self.documents[n].commit_state(state)?;
        self.tone_mapping.draft = None;
        Ok(())
    }
    pub(super) fn tone_mapping_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.tone_mapping.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d) && layer(self, d.id) == Some(e.layer);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        let old = e.op.clone();
        let old_selection = e.selection;
        egui::Window::new(if matches!(e.op,Operation::Curves(_)){"RGB and channel tone curves"}else{"Luminance gradient map"}).default_width(430.).default_pos(egui::pos2(35.,30.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame/target changed; close and reopen.");}
            match &mut e.op{
                Operation::Curves(curves)=>{
                    egui::ComboBox::from_label("Channel").selected_text(["RGB master","Red","Green","Blue"][e.channel]).show_ui(ui,|ui|{for(k,label)in["RGB master","Red","Green","Blue"].iter().enumerate(){if ui.selectable_value(&mut e.channel,k,*label).changed(){e.grab=None;}}});
                    let c=&mut curves[e.channel];ui.checkbox(&mut c.smooth,"Smooth curve");self.tone_mapping.graph=Some(graph(ui,c,&mut e.grab));
                    egui::ScrollArea::vertical().id_salt("tone-curve-points").max_height(80.).show(ui,|ui|{let mut remove=None;let len=c.points.len();for(i,p)in c.points.iter_mut().enumerate(){ui.push_id(i,|ui|{ui.horizontal(|ui|{ui.label(format!("{}",i+1));ui.add_enabled(i>0&&i+1<len,egui::DragValue::new(&mut p[0]).range(0. ..=1.).speed(0.005).prefix("In "));ui.add(egui::DragValue::new(&mut p[1]).range(0. ..=1.).speed(0.005).prefix("Out "));if i>0&&i+1<len&&ui.small_button("Remove").clicked(){remove=Some(i);}});});}if let Some(i)=remove{c.points.remove(i);e.grab=None;}});
                    ui.horizontal(|ui|{if ui.add_enabled(c.valid()&&c.points.len()<32,egui::Button::new("Add control point")).clicked(){let i=(0..c.points.len()-1).max_by(|a,b|(c.points[*a+1][0]-c.points[*a][0]).total_cmp(&(c.points[*b+1][0]-c.points[*b][0]))).unwrap();let x=(c.points[i][0]+c.points[i+1][0])/2.;let y=c.sample(x).unwrap();c.points.insert(i+1,[x,y]);}
 if ui.button("Reset channel").clicked(){*c=Curve::default();e.grab=None;}
 if ui.button("Invert channel").clicked(){c.points=vec![[0.,1.],[1.,0.]];e.grab=None;}});
                    ui.small("Drag controls or edit input/output. Master applies first, then individual channels. Overlapping input points cannot be applied.");
                }
                Operation::Gradient(ramp)=>{
                    let(r,_)=ui.allocate_exact_size(egui::vec2(360.,20.),egui::Sense::hover());if ramp.valid(){for x in 0..180{let c=ramp.sample(x as f64/179.);ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(r.left()+x as f32*2.,r.top()),egui::pos2(r.left()+(x+1) as f32*2.,r.bottom())),0.,egui::Color32::from_rgb(c[0] as u8,c[1] as u8,c[2] as u8));}}
                    egui::ComboBox::from_label("Mix colors").selected_text(ramp.space.label()).show_ui(ui,|ui|{for s in Space::ALL{ui.selectable_value(&mut ramp.space,s,s.label());}});
                    egui::ScrollArea::vertical().id_salt("tone-map-stops").max_height(160.).show(ui,|ui|{let mut remove=None;for(i,s)in ramp.stops.iter_mut().enumerate(){ui.push_id(i,|ui|{ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut s.position).range(0. ..=1.).speed(0.005));let mut rgb=[s.color[0],s.color[1],s.color[2]];ui.color_edit_button_srgb(&mut rgb);s.color=[rgb[0],rgb[1],rgb[2],255];if ui.small_button("Remove").clicked(){remove=Some(i);}});});}if let Some(i)=remove&&ramp.stops.len()>2{ramp.stops.remove(i);}});
                    ui.horizontal(|ui|{if ui.add_enabled(ramp.stops.len()<32,egui::Button::new("Add color stop")).clicked(){ramp.stops.push(Stop::new(0.5,[128,128,128,255]));ramp.stops.sort_by(|a,b|a.position.total_cmp(&b.position));}
 if ui.button("Sort stops").clicked(){ramp.stops.sort_by(|a,b|a.position.total_cmp(&b.position));}});ui.small("Map sRGB brightness to these RGB colors. Equal stop positions create a hard transition. Pixel alpha stays unchanged.");
                }
            }
            ui.checkbox(&mut e.selection,"Only within the area selection");
            if e.op!=old||e.selection!=old_selection{e.prepared=None;e.images=None;}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare tone mapping preview"));self.tone_mapping.preview=Some(r.rect);preview=r.clicked();
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}
            let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply tone mapping as one Undo"));self.tone_mapping.apply=Some(r.rect);apply=r.clicked();ui.small("Applies to the current raster's pixels. Alpha/geometry stay unchanged. Escape or close discards the preview.");
        });
        if open {
            self.tone_mapping.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_tone_mapping(ctx) {
            self.status = Some(e);
        }
        if apply && let Err(e) = self.apply_tone_mapping() {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tone_curve_mouse_graph_preview_cancel_stale_apply_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
