use super::*;
use drawer_core::layer_comps::{self as core, Command};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    save: Option<egui::Rect>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    selected: Option<u64>,
    name: String,
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn begin_layer_comps(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let c = d.scene.layer_comps.comps.first();
        self.layer_comps.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected: c.map(|c| c.id),
            name: c.map_or_else(|| "Comp".into(), |c| c.name.clone()),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn layer_comp_command(&mut self, command: Command) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .layer_comps
            .draft
            .as_mut()
            .ok_or("Comp editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen layer comps".into());
        }
        let p = core::prepare(d, command)?;
        self.documents[n].commit_state(p.state)?;
        e.selected = p.selected;
        e.revision = self.documents[n].revision;
        e.prepared = None;
        e.images = None;
        Ok(())
    }
    fn preview_layer_comp(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .layer_comps
            .draft
            .as_ref()
            .ok_or("Comp editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen layer comps".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(128 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Comp preview exceeds workspace budget".into());
        }
        let p = core::prepare(d, Command::Apply(e.selected.ok_or("Choose a saved comp")?))?;
        let after = drawer_core::layer_merge::preview(&p.state, e.frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "comp-before"),
            super::layer_merge::image(ctx, &after, "comp-after"),
        ];
        let e = self.layer_comps.draft.as_mut().unwrap();
        e.prepared = Some(p);
        e.images = Some(images);
        Ok(())
    }
    fn apply_layer_comp(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .layer_comps
            .draft
            .as_mut()
            .ok_or("Comp editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen layer comps".into());
        }
        let p = e.prepared.take().ok_or("Preview the comp first")?;
        let message = format!(
            "Comp applied: {} layers changed, {} missing, {} unrecorded layers preserved",
            p.changed,
            p.missing.len(),
            p.unrecorded
        );
        self.documents[n].commit_state(p.state)?;
        self.status = Some(message);
        self.layer_comps.draft = None;
        Ok(())
    }
    pub(super) fn layer_comps_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_comps.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut command, mut preview, mut apply, mut cancel) = (None, false, false, false);
        egui::Window::new("Layer comps").open(&mut open).default_pos(egui::pos2(25.,20.)).default_width(480.).collapsible(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen layer comps.");return;}
            ui.label("Save each layer's own visibility, opacity and blend mode. New layers retain their current settings.");
            egui::ScrollArea::vertical().max_height(110.).show(ui,|ui|{for c in &d.scene.layer_comps.comps{
                if ui.selectable_label(e.selected==Some(c.id),&c.name).clicked(){e.selected=Some(c.id);e.name=c.name.clone();e.prepared=None;e.images=None;}
            }});
            ui.text_edit_singleline(&mut e.name);
            ui.horizontal(|ui|{
                let r=ui.button("Save new comp");self.layer_comps.save=Some(r.rect);if r.clicked(){command=Some(Command::Capture{name:e.name.clone(),replace:None});}
                if ui.add_enabled(e.selected.is_some(),egui::Button::new("Update saved states")).clicked(){command=Some(Command::Capture{name:e.name.clone(),replace:e.selected});}
                if let Some(id)=e.selected{
                    if ui.button("Rename").clicked(){command=Some(Command::Rename{id,name:e.name.clone()});}
                    if ui.button("Delete").clicked(){command=Some(Command::Delete(id));}
                }
            });
            let r=ui.add_enabled(e.selected.is_some(),egui::Button::new("Preview comp"));self.layer_comps.preview=Some(r.rect);preview=r.clicked();
            if let Some(p)=&e.prepared{
                ui.label(format!("Changed: {} / missing: {} / unrecorded kept: {}",p.changed,p.missing.len(),p.unrecorded));
                egui::ScrollArea::vertical().max_height(65.).show(ui,|ui|{for key in &p.missing{ui.small(format!("Missing layer: {key:?}"));}});
            }
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current",&images[0]),("After comp",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some(),egui::Button::new("Apply comp"));self.layer_comps.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.layer_comps.draft = Some(e);
        }
        let result = if let Some(command) = command {
            self.layer_comp_command(command)
        } else if preview {
            self.preview_layer_comp(ctx)
        } else if apply {
            self.apply_layer_comp()
        } else {
            Ok(())
        };
        if let Err(e) = result {
            self.status = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_comps_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    let mut time = 0.;
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
    };
    app.execute_command(&ctx, "layer-comps");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let r = app.layer_comps.save.ok_or("Comp save button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].scene.layer_comps.comps.len() != 1
        || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!("Comp UI capture/Undo failed: {:?}", app.status));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before || !app.documents[0].redo()
    {
        return Err("Comp capture Undo/Redo".into());
    }
    let state = drawer_core::layer_properties::prepare_patch(
        &app.documents[0],
        &[drawer_core::layer_properties::Target::Raster(1)],
        drawer_core::layer_properties::Patch {
            opacity: Some(25),
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    app.documents[0].commit_state(state)?;
    app.execute_command(&ctx, "layer-comps");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let r = app
        .layer_comps
        .preview
        .ok_or("Comp preview button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != original
        || app
            .layer_comps
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!(
            "Comp preview mutated source or failed: {:?}",
            app.status
        ));
    }
    step(&mut app, &mut time, vec![]);
    let r = app.layer_comps.apply.ok_or("Comp apply missing")?;
    click(&mut app, &mut time, r);
    if app.layer_comps.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || drawer_core::layer_properties::properties(
            &app.documents[0],
            drawer_core::layer_properties::Target::Raster(1),
        )
        .opacity
            != 100
    {
        return Err(format!("Comp UI apply/one Undo failed: {:?}", app.status));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Comp UI Undo".into());
    }
    app.execute_command(&ctx, "layer-comps");
    app.preview_layer_comp(&ctx)?;
    app.documents[0].current_frame += 1;
    let snapshot = app.documents[0].snapshot();
    if app.apply_layer_comp().is_ok() || app.documents[0].snapshot() != snapshot {
        return Err("Stale frame comp applied".into());
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
    if app.layer_comps.draft.is_some() {
        return Err("Comp Escape did not close".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn comp_capture_preview_apply_undo_stale_escape() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
