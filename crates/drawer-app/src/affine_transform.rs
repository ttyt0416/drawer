use super::*;
use drawer_core::{
    affine_transform as core, interpolation::Filter, layer_catalog as catalog, layer_search::Key,
    vector_scope::Transform,
};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::affine_transform_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::affine_transform_verify::fixture()?.snapshot())?;
    let mut time = 0.;
    let frame = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        crate::input_replay::frame(&ctx, app, *time, events);
        app.wait_transform_preview(&ctx, time)
            .expect("Transform preview timeout");
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
    app.execute_command(&ctx, "scale-rotate");
    let e = app
        .affine_transform
        .draft
        .as_mut()
        .ok_or("Scale/rotate command did not open")?;
    e.pixels = true;
    e.filter = Filter::Nearest;
    e.settings = Transform {
        pivot: [1., 1.],
        scale: [-1., 2.],
        translation: [4., 1.],
        ..Default::default()
    };
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app
        .affine_transform
        .preview
        .ok_or("Transform preview button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app
            .affine_transform
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
    {
        return Err(format!(
            "Transform mouse preview/original isolation: {:?}",
            app.status
        ));
    }
    let file = root.join("unconfirmed-transform.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != original
    {
        return Err("Unconfirmed transform was saved".into());
    }
    app.documents[0].view.zoom = 2.;
    app.documents[0].view.angle = 90.;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app
        .affine_transform
        .apply
        .ok_or("Transform Apply button missing")?;
    if !ctx.content_rect().contains(r.center()) {
        return Err(format!("Transform Apply is outside viewport: {r:?}"));
    }
    click(&mut app, &mut time, r);
    if app.affine_transform.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].layers[0].pixel(8, 8, 4, 2) != [220, 20, 30, 255]
    {
        return Err(format!("Transform mouse Apply/one Undo: {:?}", app.status));
    }
    let after = app.documents[0].snapshot();
    let native = root.join("affine-transform-app.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != after
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Transform app native/Undo".into());
    }
    app.execute_command(&ctx, "scale-rotate");
    app.affine_transform
        .draft
        .as_mut()
        .unwrap()
        .settings
        .degrees = 45.;
    app.preview_affine_transform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
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
    if app.affine_transform.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Transform Escape committed preview".into());
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
    app.execute_command(&ctx, "scale-rotate");
    app.affine_transform
        .draft
        .as_mut()
        .unwrap()
        .settings
        .degrees = 45.;
    app.preview_affine_transform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    app.documents[0].current_frame += 1;
    if app.apply_affine_transform().is_ok() {
        return Err("Stale transform frame applied".into());
    }
    app.documents[0].current_frame -= 1;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Vector(999),
    );
    if app.apply_affine_transform().is_ok() {
        return Err("Stale transform target applied".into());
    }
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    app.affine_transform.draft.as_mut().unwrap().settings.scale = [0., 1.];
    if app.preview_affine_transform(&ctx).is_ok()
        || app
            .affine_transform
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .is_some()
        || app.apply_affine_transform().is_ok()
        || app.documents[0].snapshot() != original
    {
        return Err("Failed transform preview left previous plan applicable".into());
    }
    Ok(())
}
pub(super) struct Draft {
    pub(super) id: u64,
    pub(super) revision: u64,
    pub(super) frame: u32,
    pub(super) target: Key,
    pub(super) pixels: bool,
    pub(super) bake: bool,
    pub(super) settings: Transform,
    pub(super) filter: Filter,
    pub(super) prepared: Option<core::Prepared>,
    pub(super) images: Option<[egui::TextureHandle; 2]>,
    pub(super) quality: crate::transform_review::Quality,
    pub(super) aspect: bool,
}
fn target(app: &Drawer, d: &Document) -> Option<Key> {
    catalog::from_thumbnail(
        d,
        app.panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
    )
}
impl Draft {
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
}
impl Drawer {
    pub(super) fn begin_affine_transform(&mut self) -> Result<(), String> {
        self.finish_stroke();
        self.transform_begin_guard(0)?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let key = target(self, d).ok_or("Choose a layer/object")?;
        let bounds = catalog::bounds(d, key).unwrap_or(drawer_core::scene::Rect {
            min: [0.; 2],
            max: [d.width as f64, d.height as f64],
        });
        self.affine_transform.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target: key,
            quality: Default::default(),
            aspect: true,
            pixels: false,
            bake: false,
            settings: Transform {
                pivot: std::array::from_fn(|k| (bounds.min[k] + bounds.max[k]) / 2.),
                ..Default::default()
            },
            filter: Filter::Bilinear,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    pub(super) fn preview_affine_transform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.affine_transform.draft {
            e.prepared = None;
            e.images = None;
        }
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .affine_transform
            .draft
            .as_ref()
            .ok_or("Transform window closed")?;
        if !((d.id, d.revision, d.current_frame) == (e.id, e.revision, e.frame)
            && target(self, d) == Some(e.target))
        {
            return Err("Page/frame/target changed; reopen the transform".into());
        }
        self.start_transform_preview(ctx, 0)
    }
    pub(super) fn apply_affine_transform(&mut self) -> Result<(), String> {
        self.transform_apply_guard(0)?;
        let n = self.active.ok_or("Open a page first")?;
        let current = target(self, &self.documents[n]);
        let e = self
            .affine_transform
            .draft
            .as_mut()
            .ok_or("Transform window closed")?;
        if !e.fresh(&self.documents[n]) || current != Some(e.target) {
            return Err("Page/frame/target changed; reopen and preview again".into());
        }
        let p = e
            .prepared
            .take()
            .ok_or("Prepare the transform preview first")?;
        self.documents[n].commit_state(p.state)?;
        if let Some(t) = catalog::thumbnail(&self.documents[n], p.selected) {
            self.panels.targets.insert(self.documents[n].id, t);
        }
        self.affine_transform.draft = None;
        Ok(())
    }
    pub(super) fn affine_transform_window(&mut self, ctx: &egui::Context) {
        let working = self.transform_working();
        let Some(mut e) = self.affine_transform.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d) && target(self, d) == Some(e.target);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        let mut cancel = false;
        egui::Window::new("Scale and rotate").default_width(465.).default_pos(egui::pos2(35.,30.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame/target changed; close and reopen this window.");}
            let mut changed=false;
            changed|=crate::transform_review::quality(ui,&mut e.quality);
            if working {ui.label("Calculating transform — close or Escape to cancel");}
            egui::ScrollArea::vertical().id_salt("affine-controls").max_height(240.).show(ui,|ui|{
            changed|=ui.checkbox(&mut e.pixels,"Transform selected pixels in the current raster").changed();
            changed|=crate::transform_review::scales(ui,&mut e.settings.scale,&mut e.aspect,-1e4..=1e4).0;
            changed|=ui.add(egui::DragValue::new(&mut e.settings.degrees).range(-36000. ..=36000.).prefix("Degrees: ")).changed();
            ui.horizontal(|ui|{for(k,label)in["Pivot X: ","Pivot Y: "].iter().enumerate(){changed|=ui.add(egui::DragValue::new(&mut e.settings.pivot[k]).range(-1e6..=1e6).prefix(*label)).changed();}});
            ui.horizontal(|ui|{for(k,label)in["Move X: ","Move Y: "].iter().enumerate(){changed|=ui.add(egui::DragValue::new(&mut e.settings.translation[k]).range(-1e6..=1e6).prefix(*label)).changed();}});
            if ui.button("Use canvas center as pivot").clicked(){e.settings.pivot=[d.width as f64/2.,d.height as f64/2.];changed=true;}
            ui.small("Negative scale flips that axis. Zero scale is rejected. Coordinates and angles use the document; the preview always starts from the original.");
            if e.pixels{ui.horizontal(|ui|{for(f,label)in[(Filter::Nearest,"Hard edges"),(Filter::Bilinear,"Smooth edges")]{changed|=ui.radio_value(&mut e.filter,f,label).changed();}});ui.small("Only the selected coverage moves; its area selection follows. The current layer's mask stays fixed. Transparent edges use premultiplied interpolation.");}
            else{changed|=ui.checkbox(&mut e.bake,"Convert unsupported text/tone/3D/animation sources to a paintable raster on Apply").changed();ui.small("This conversion is optional and included in the preview. Undo restores the editable source. Linked layers transform together; incompatible linked members reject the operation.");}
            });
            if changed{e.prepared=None;e.images=None;}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare transform preview"));self.affine_transform.preview=Some(r.rect);preview=r.clicked();cancel=ui.button("Cancel transform").clicked();
            if let Some(p)=&e.prepared{egui::ScrollArea::vertical().id_salt("affine-notes").max_height(72.).show(ui,|ui|{for note in &p.notes{ui.small(note);}});}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(120.,120.)));});}});}
            let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply transform as one Undo"));self.affine_transform.apply=Some(r.rect);apply=r.clicked();ui.small("Escape or close discards the transform.");
        });
        if open && !cancel {
            self.affine_transform.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_affine_transform(ctx) {
            self.status = Some(e);
        }
        if apply && let Err(e) = self.apply_affine_transform() {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn affine_mouse_preview_cancel_stale_apply_save_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
