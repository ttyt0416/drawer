use super::*;
use drawer_core::{vector_cleanup as core, vector_scope::Picked};
fn raster_target(target: Option<&drawer_core::thumbnail::Target>) -> Option<usize> {
    match target {
        None => Some(0),
        Some(drawer_core::thumbnail::Target::Raster(n)) => Some(*n),
        _ => None,
    }
}
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::vector_cleanup_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 24,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::vector_cleanup_verify::fixture()?.snapshot())?;
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
    app.execute_command(&ctx, "raster-trace");
    app.vector_cleanup
        .draft
        .as_mut()
        .ok_or("Trace command did not open")?
        .settings
        .colors = 2;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app
        .vector_cleanup
        .preview
        .ok_or("Trace preview button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != original
        || app.documents[0].history_counts() != counts
        || app
            .vector_cleanup
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none() || e.images.is_none())
    {
        return Err(format!(
            "Trace mouse preview/source isolation: {:?}",
            app.status
        ));
    }
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app
        .vector_cleanup
        .apply
        .ok_or("Trace Apply button missing")?;
    click(&mut app, &mut time, r);
    if app.vector_cleanup.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].layers != original.layers
    {
        return Err("Trace mouse Apply lost source/one Undo".into());
    }
    let imported = app.documents[0].snapshot();
    let native = root.join("vector-cleanup-app.drawer");
    drawer_core::files::save(&mut app.documents[0], &native).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != imported
    {
        return Err("Vector cleanup app native roundtrip".into());
    }
    app.execute_command(&ctx, "vector-remove-short");
    app.vector_cleanup.draft.as_mut().unwrap().all = true;
    app.vector_cleanup.draft.as_mut().unwrap().length = 1000.;
    app.preview_vector_cleanup(&ctx)?;
    if app.documents[0].snapshot() != imported
        || app
            .vector_cleanup
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .changed
            != 2
    {
        return Err("All-layer short stroke preview/source changed".into());
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
    if app.vector_cleanup.draft.is_some() || app.documents[0].snapshot() != imported {
        return Err("Cleanup Escape changed source".into());
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
    app.execute_command(&ctx, "vector-remove-short");
    app.vector_cleanup.draft.as_mut().unwrap().all = true;
    app.vector_cleanup.draft.as_mut().unwrap().length = 1000.;
    app.preview_vector_cleanup(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_vector_cleanup().is_ok() {
        return Err("Cleanup stale frame applied".into());
    }
    app.documents[0].current_frame -= 1;
    app.apply_vector_cleanup()?;
    if !app.documents[0].scene.lines.layers[0].paths.is_empty()
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != imported
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err("Trace/cleanup independent one Undo".into());
    }
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    app.execute_command(&ctx, "raster-trace");
    app.preview_vector_cleanup(&ctx)?;
    app.vector_cleanup.draft.as_mut().unwrap().settings.colors = 0;
    if app.preview_vector_cleanup(&ctx).is_ok()
        || app
            .vector_cleanup
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .is_some()
        || app.apply_vector_cleanup().is_ok()
        || app.documents[0].snapshot() != original
    {
        return Err("Failed preview left an applicable prior plan".into());
    }
    Ok(())
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    raster: Option<usize>,
    scope: Vec<Picked>,
    all: bool,
    settings: core::Trace,
    length: f64,
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Draft {
    fn fresh(&self, d: &Document) -> bool {
        (self.id, self.revision, self.frame) == (d.id, d.revision, d.current_frame)
    }
}
impl Drawer {
    pub(super) fn begin_vector_cleanup(&mut self, trace: bool) -> Result<(), String> {
        self.finish_stroke();
        let scope = if trace {
            vec![]
        } else {
            self.vector_cleanup_scope()?
        };
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let raster = if trace {
            Some(
                raster_target(self.panels.targets.get(&d.id))
                    .ok_or("Select the source raster layer first")?,
            )
        } else {
            None
        };
        self.vector_cleanup.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            raster,
            scope,
            all: false,
            settings: Default::default(),
            length: 2.,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_vector_cleanup(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let scope = self.vector_cleanup_scope()?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .vector_cleanup
            .draft
            .as_mut()
            .ok_or("Open vector conversion/cleanup first")?;
        e.prepared = None;
        e.images = None;
        if !e.fresh(d) || e.raster.is_none() && !e.all && e.scope != scope {
            return Err("Page/frame/selection changed; reopen the window".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(64 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Vector preview exceeds workspace memory".into());
        }
        let prepared = if let Some(layer) = e.raster {
            if raster_target(self.panels.targets.get(&d.id)) != Some(layer) {
                return Err("Raster target changed; reopen the trace window".into());
            }
            core::trace(d, layer, e.settings)?
        } else {
            core::remove_short(
                d,
                &if e.all { core::all(d) } else { e.scope.clone() },
                e.length,
            )?
        };
        let after = crate::drawing::bucket::preview(d, prepared.state.clone())?;
        e.images = Some([
            crate::layer_merge::image(ctx, d, "vector-cleanup-before"),
            crate::layer_merge::image(ctx, &after, "vector-cleanup-after"),
        ]);
        e.prepared = Some(prepared);
        Ok(())
    }
    fn apply_vector_cleanup(&mut self) -> Result<(), String> {
        let scope = self.vector_cleanup_scope()?;
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .vector_cleanup
            .draft
            .as_mut()
            .ok_or("Vector window closed")?;
        if !e.fresh(d)
            || e.raster.is_none() && !e.all && e.scope != scope
            || e.raster
                .is_some_and(|n| raster_target(self.panels.targets.get(&d.id)) != Some(n))
        {
            return Err("Page/frame/target/selection changed; reopen and prepare again".into());
        }
        let p = e.prepared.take().ok_or("Prepare the preview first")?;
        let changed = p.changed;
        if changed > 0 {
            d.commit_state(p.state)?;
        }
        if let Some(id) = p.layer {
            self.panels
                .targets
                .insert(d.id, drawer_core::thumbnail::Target::Vector(id));
        }
        self.vector_cleanup.draft = None;
        self.status = Some(format!(
            "{changed} vector contours/spans processed; Undo restores the source"
        ));
        Ok(())
    }
    pub(super) fn vector_cleanup_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.vector_cleanup.draft.take() else {
            return;
        };
        let Some(d) = self.active.map(|n| &self.documents[n]) else {
            return;
        };
        let fresh = e.fresh(d);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply) = (false, false);
        self.vector_cleanup.preview = None;
        self.vector_cleanup.apply = None;
        egui::Window::new(if e.raster.is_some(){"Trace raster as editable vectors"}else{"Remove short vector strokes or selected spans"}).default_width(470.).default_pos(egui::pos2(32.,60.)).open(&mut open).show(ctx,|ui|{
            let mut changed=false;
            if !fresh {ui.colored_label(egui::Color32::YELLOW,"Page/frame changed; close and reopen this window.");}
            if e.raster.is_some() {
                ui.label("Trace the selected raster's stored pixels on this canvas. Keep the source layer and add a new filled vector layer.");
                changed|=ui.add(egui::DragValue::new(&mut e.settings.alpha).range(0..=255).prefix("Minimum alpha: ")).changed();
                changed|=ui.add(egui::DragValue::new(&mut e.settings.brightness).range(0..=255).prefix("Maximum brightness: ")).changed();
                changed|=ui.add(egui::DragValue::new(&mut e.settings.colors).range(1..=16).prefix("Color groups: ")).changed();
                changed|=ui.add(egui::DragValue::new(&mut e.settings.error).range(0. ..=1024.).speed(0.1).prefix("Maximum contour error (pixels): ")).changed();
                ui.small("0 error follows exact pixel edges, including holes and semitransparent colors. Colors are clustered in RGBA. Simplification retains contour topology. Limits: 1M pixels, 65536 input colors/edges, 4096 nodes per path and 8192 total nodes.");
            }else{
                changed|=ui.checkbox(&mut e.all,"All eligible vector layers").changed();
                changed|=ui.add(egui::DragValue::new(&mut e.length).range(0. ..=1e9).speed(0.1).prefix("Remove lengths strictly below (document pixels): ")).changed();
                ui.small("Otherwise process the captured vector selection across layers: whole selected strokes, or connected spans between adjacent selected controls. Keep the remaining geometry/pressure. Hidden, locked and linked layers are excluded from all-layer processing. Equal lengths remain.");
            }
            if changed {e.prepared=None;e.images=None;}
            let r=ui.add_enabled(fresh,egui::Button::new("Prepare conversion/cleanup preview"));self.vector_cleanup.preview=Some(r.rect);preview=r.clicked();
            if let Some(p)=&e.prepared {ui.label(format!("{} contours/spans will change",p.changed));}
            if let Some(images)=&e.images {ui.horizontal(|ui|{for(label,image)in["Before","After"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(120.,120.)));});}});}
            let r=ui.add_enabled(fresh && e.prepared.is_some(),egui::Button::new("Apply as one Undo"));self.vector_cleanup.apply=Some(r.rect);apply=r.clicked();
            ui.small("Escape or close discards the preview. Reopen after changing the page, frame, target or vector selection.");
        });
        if open {
            self.vector_cleanup.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_vector_cleanup(ctx) {
            self.status = Some(e);
        }
        if apply && let Err(e) = self.apply_vector_cleanup() {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn vector_trace_cleanup_mouse_preview_cancel_stale_apply_native_and_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
