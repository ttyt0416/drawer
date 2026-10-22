use super::*;
use drawer_core::color_corrections::{self as core, Operation as Op};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: usize,
    op: Op,
    selection: bool,
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
    fn invalidate(&mut self) {
        self.prepared = None;
        self.images = None;
    }
}
impl Drawer {
    pub(super) fn begin_color_corrections(&mut self, op: Op) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let layer = layer(self, d.id).ok_or("Select a raster layer")?;
        drawer_core::layer_properties::paintable(d, layer)?;
        self.color_corrections.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            op,
            selection: false,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_color_corrections(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = layer(self, d.id);
        let e = self
            .color_corrections
            .draft
            .as_mut()
            .ok_or("Correction window closed")?;
        e.invalidate();
        if !e.fresh(d) || target != Some(e.layer) {
            return Err("Page/frame/target changed; reopen color correction".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(5))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Correction preview workspace limit".into());
        }
        let state = core::prepare(d, e.layer, e.selection, e.op)?;
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "native-correction-before"),
            crate::layer_merge::image(ctx, &after, "native-correction-after"),
        ]);
        e.prepared = Some(state);
        self.status = None;
        Ok(())
    }
    fn apply_color_corrections(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let target = layer(self, self.documents[n].id);
        let e = self
            .color_corrections
            .draft
            .as_mut()
            .ok_or("Correction window closed")?;
        if !e.fresh(&self.documents[n]) || target != Some(e.layer) {
            return Err("Page/frame/target changed; reopen color correction".into());
        }
        let state = e
            .prepared
            .take()
            .ok_or("Prepare a valid correction preview first")?;
        self.documents[n].commit_state(state)?;
        self.color_corrections.draft = None;
        self.status = None;
        Ok(())
    }
    pub(super) fn color_corrections_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.color_corrections.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d) && layer(self, d.id) == Some(e.layer);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        egui::Window::new("Document RGB correction").default_width(470.).default_pos(egui::pos2(30.,45.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame/target changed; close and reopen.");}
            let mut changed=false;egui::ComboBox::from_id_salt("native-rgb-correction").selected_text(e.op.label()).show_ui(ui,|ui|{for op in Op::defaults(){if ui.selectable_label(std::mem::discriminant(&op)==std::mem::discriminant(&e.op),op.label()).clicked(){e.op=op;changed=true;}}});
            match &mut e.op{
                Op::Levels{input,output,gamma}=>{
                    ui.horizontal(|ui|{ui.label("Input black / white");for v in input{changed|=ui.add(egui::DragValue::new(v).range(0. ..=1.).speed(0.01)).changed();}});
                    ui.horizontal(|ui|{ui.label("Output black / white");for v in output{changed|=ui.add(egui::DragValue::new(v).range(0. ..=1.).speed(0.01)).changed();}});
                    changed|=ui.add(egui::Slider::new(gamma,0.01..=10.).text("Midtone gamma")).changed();
                }
                Op::Balance(values)=>{for(v,label)in values.iter_mut().zip(["Cyan − / Red +","Magenta − / Green +","Yellow − / Blue +"]){changed|=ui.add(egui::Slider::new(v,-1. ..=1.).text(label)).changed();}}
                Op::Posterize(n)=>{changed|=ui.add(egui::Slider::new(n,2..=256).text("Levels per RGB channel")).changed();}
                Op::Threshold(t)=>{changed|=ui.add(egui::Slider::new(t,0. ..=1.).text("RGB weighted luminance threshold")).changed();}
                Op::Invert=>{ui.label("Invert each document RGB component: black ↔ white. Alpha stays unchanged.");}
            }
            changed|=ui.checkbox(&mut e.selection,"Only within the area selection").changed();
            ui.small("Works in the document RGB values; its profile is retained. Pixels with zero alpha keep their hidden colors. Layer masks stay fixed and continue to control display.");
            if changed{e.invalidate();}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare RGB correction preview"));self.color_corrections.preview=Some(r.rect);preview=r.clicked();
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(140.,110.)));});}});}
            let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply RGB correction as one Undo"));self.color_corrections.apply=Some(r.rect);apply=r.clicked();
            ui.small("Escape/close discards the preview. Use Inversion correction layer from Layers… for live inversion with masks and opacity.");
        });
        if open {
            self.color_corrections.draft = Some(e);
        }
        if preview && let Err(err) = self.preview_color_corrections(ctx) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.apply_color_corrections() {
            self.status = Some(err);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::color_corrections_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 2,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::color_corrections_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
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
    for command in [
        "color-levels",
        "color-balance",
        "color-posterize",
        "color-threshold",
        "invert-pixels",
    ] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, command);
        let e = app
            .color_corrections
            .draft
            .as_mut()
            .ok_or("RGB correction command missing")?;
        match &mut e.op {
            Op::Levels { input, .. } => *input = [0.1, 0.9],
            Op::Balance(v) => *v = [0.2, -0.1, 0.3],
            _ => {}
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .color_corrections
            .preview
            .ok_or("RGB preview button missing")?;
        click(&mut app, &mut time, r);
        if app
            .color_corrections
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
            || app.documents[0].snapshot() != original
        {
            return Err(format!("RGB correction mouse preview: {:?}", app.status));
        }
        let pending = root.join("unconfirmed-rgb.drawer");
        app.save_to(0, &pending)?;
        if drawer_core::files::open(&pending, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != original
        {
            return Err("Unconfirmed RGB correction was stored".into());
        }
        for _ in 0..3 {
            frame(&mut app, &mut time, vec![]);
        }
        let r = app
            .color_corrections
            .apply
            .ok_or("RGB Apply button missing")?;
        click(&mut app, &mut time, r);
        if app.color_corrections.draft.is_some()
            || app.documents[0].snapshot() == original
            || app.documents[0].history_counts().0 != counts.0 + 1
            || !app.documents[0].undo()
            || app.documents[0].snapshot() != original
        {
            return Err(format!(
                "RGB correction Apply/Undo: {command}; {:?}",
                app.status
            ));
        }
    }
    app.execute_command(&ctx, "invert-correction-layer");
    if app.adjustments.draft.is_none() {
        return Err("Inversion correction layer command missing".into());
    }
    app.preview_adjustment(&ctx)?;
    app.apply_adjustment()?;
    if app.documents[0].layers.len() != original.layers.len() + 1
        || app.documents[0].composite_pixel(0, 0) != [255; 4]
        || app.documents[0].layers[0] != original.layers[0]
    {
        return Err("Live RGB inversion command did not preserve source".into());
    }
    let native = root.join("color-corrections-app.drawer");
    app.save_to(0, &native)?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Live inversion app native/Undo failed".into());
    }
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    app.execute_command(&ctx, "color-levels");
    app.preview_color_corrections(&ctx)?;
    app.color_corrections.draft.as_mut().unwrap().op = Op::Levels {
        input: [1., 0.],
        output: [0., 1.],
        gamma: 1.,
    };
    if app.preview_color_corrections(&ctx).is_ok()
        || app
            .color_corrections
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .is_some()
        || app.apply_color_corrections().is_ok()
    {
        return Err("Invalid levels retained previous plan".into());
    }
    app.color_corrections.draft = None;
    app.execute_command(&ctx, "invert-pixels");
    app.preview_color_corrections(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_color_corrections().is_ok() {
        return Err("Stale RGB correction applied".into());
    }
    app.documents[0].current_frame -= 1;
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
    if app.color_corrections.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("RGB Escape committed draft".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_rgb_commands_mouse_preview_apply_layers_cancel_invalid_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
