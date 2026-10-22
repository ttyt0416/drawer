use super::*;
use drawer_core::gaussian as core;
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::gaussian_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 5,
        height: 4,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::gaussian_verify::fixture()?.snapshot())?;
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
    app.execute_command(&ctx, "gaussian-blur");
    app.gaussian
        .draft
        .as_mut()
        .ok_or("Gaussian command did not open")?
        .radius = 2;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app
        .gaussian
        .preview
        .ok_or("Gaussian preview button missing")?;
    click(&mut app, &mut time, r);
    if app
        .gaussian
        .draft
        .as_ref()
        .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
        || app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
    {
        return Err(format!(
            "Gaussian mouse preview/source isolation: {:?}",
            app.status
        ));
    }
    let native = root.join("unconfirmed-gaussian.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != original
    {
        return Err("Unconfirmed Gaussian was saved".into());
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.gaussian.apply.ok_or("Gaussian Apply button missing")?;
    click(&mut app, &mut time, r);
    if app.gaussian.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].snapshot() == original
    {
        return Err("Gaussian Apply/one Undo".into());
    }
    let native = root.join("gaussian-app.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != app.documents[0].snapshot()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Gaussian app native/Undo".into());
    }
    let counts = app.documents[0].history_counts();
    let dirty = app.documents[0].dirty;
    app.execute_command(&ctx, "gaussian-blur");
    app.gaussian.draft.as_mut().unwrap().radius = 0;
    app.preview_gaussian(&ctx)?;
    app.apply_gaussian()?;
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app.documents[0].dirty != dirty
    {
        return Err("Zero radius destroyed redo, dirty flag or source".into());
    }
    app.execute_command(&ctx, "gaussian-blur");
    app.preview_gaussian(&ctx)?;
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
    if app.gaussian.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Gaussian Escape committed preview".into());
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
    app.execute_command(&ctx, "gaussian-blur");
    app.preview_gaussian(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_gaussian().is_ok() {
        return Err("Stale Gaussian frame applied".into());
    }
    app.documents[0].current_frame -= 1;
    app.gaussian.draft.as_mut().unwrap().radius = 65;
    if app.preview_gaussian(&ctx).is_ok()
        || app.gaussian.draft.as_ref().unwrap().prepared.is_some()
        || app.apply_gaussian().is_ok()
        || app.documents[0].snapshot() != original
    {
        return Err("Failed Gaussian preview retained prior plan".into());
    }
    Ok(())
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    layer: usize,
    radius: u8,
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
}
impl Drawer {
    pub(super) fn begin_gaussian(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let layer = layer(self, d.id).ok_or("Select a raster layer")?;
        self.gaussian.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            layer,
            radius: 3,
            selection: false,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_gaussian(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = layer(self, d.id);
        let e = self
            .gaussian
            .draft
            .as_mut()
            .ok_or("Gaussian window closed")?;
        e.prepared = None;
        e.images = None;
        if !e.fresh(d) || target != Some(e.layer) {
            return Err("Page/frame/target changed; reopen Gaussian blur".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(5))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Gaussian preview exceeds workspace memory".into());
        }
        let state = core::prepare(d, e.layer, e.radius, e.selection)?;
        if used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Gaussian output exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "gaussian-before"),
            crate::layer_merge::image(ctx, &after, "gaussian-after"),
        ]);
        e.prepared = Some(state);
        self.status = None;
        Ok(())
    }
    fn apply_gaussian(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let target = layer(self, self.documents[n].id);
        let e = self
            .gaussian
            .draft
            .as_mut()
            .ok_or("Gaussian window closed")?;
        if !e.fresh(&self.documents[n]) || target != Some(e.layer) {
            return Err("Page/frame/target changed; preview Gaussian blur again".into());
        }
        let state = e.prepared.take().ok_or("Prepare Gaussian preview first")?;
        self.documents[n].commit_state(state)?;
        self.gaussian.draft = None;
        self.status = None;
        Ok(())
    }
    pub(super) fn gaussian_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.gaussian.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d) && layer(self, d.id) == Some(e.layer);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        egui::Window::new("Gaussian blur").default_width(430.).default_pos(egui::pos2(35.,60.)).open(&mut open).show(ctx,|ui|{
            if !fresh{ui.colored_label(egui::Color32::YELLOW,"Page/frame/target changed; close and reopen.");}
            let mut changed=ui.add(egui::Slider::new(&mut e.radius,0..=64).text("Blur radius (document pixels)")).changed();changed|=ui.checkbox(&mut e.selection,"Only within the area selection").changed();
            ui.small("The preview always starts from the original. Transparent colors are weighted by alpha to avoid black borders. Radius0 leaves the stored pixels unchanged.");
            if drawer_core::clipping::alpha_locked(d,e.layer){ui.small("Layer transparency is protected: blur colors inside the existing alpha, keeping its coverage and bounds.");}else{ui.small("The layer grows by the radius around its stored edges; pixels outside the canvas remain in the layer.");}
            if changed{e.prepared=None;e.images=None;}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare Gaussian blur preview"));self.gaussian.preview=Some(r.rect);preview=r.clicked();
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(120.,120.)));});}});}
            let r=ui.add_enabled(fresh&&e.prepared.is_some(),egui::Button::new("Apply Gaussian blur as one Undo"));self.gaussian.apply=Some(r.rect);apply=r.clicked();ui.small("Escape or close discards the preview. Keeps8/16-bit storage and the document profile. Large radius/source combinations can exceed the processing limit.");
        });
        if open {
            self.gaussian.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_gaussian(ctx) {
            self.status = Some(e);
        }
        if apply && let Err(e) = self.apply_gaussian() {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gaussian_mouse_preview_apply_zero_cancel_stale_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
