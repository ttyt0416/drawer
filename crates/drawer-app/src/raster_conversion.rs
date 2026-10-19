use super::*;
use drawer_core::{
    layer_catalog as catalog,
    layer_search::{Expression, Key},
    raster_conversion::{self as core, Action},
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::raster_conversion_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::raster_conversion_verify::mixed()?.snapshot())?;
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
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Source(Key::Object(1)),
    );
    app.execute_command(&ctx, "raster-conversion");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .raster_conversion
        .preview
        .ok_or("Raster preview missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].snapshot() != original
        || app
            .raster_conversion
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!(
            "Raster UI preview changed page or failed: {:?}",
            app.status
        ));
    }
    let r = app.raster_conversion.apply.ok_or("Raster apply missing")?;
    click(&mut app, &mut time, r);
    if app.raster_conversion.draft.is_some() || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!("Raster UI apply/one Undo: {:?}", app.status));
    }
    let selected =
        catalog::from_thumbnail(&app.documents[0], app.panels.targets[&app.documents[0].id])
            .ok_or("Raster result not selected")?;
    let Key::Layer(drawer_core::layer_properties::Target::Raster(id)) = selected else {
        return Err("Converted layer is not raster storage".into());
    };
    let n = drawer_core::layers::index(&app.documents[0], id).unwrap();
    let rasterized = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
    {
        return Err("Raster UI Undo did not restore source".into());
    }
    app.set_tool(BrushTip::PixelPen);
    app.panels.settings.brush_size = 1.;
    app.workspaces.store.current.pressure.tools[6].size = false;
    app.workspaces.store.current.pressure.tools[6].opacity = false;
    app.workspaces.store.current.input.profiles[6].minimum_size = 1.;
    app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.;
    app.workspaces.store.current.input.profiles[6]
        .pixel
        .pressure = false;
    app.choose_color([40, 100, 160, 255]);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    for phase in [
        drawer_core::pointer_input::Phase::Down,
        drawer_core::pointer_input::Phase::Up,
    ] {
        let event = super::precision::pointer(&ctx, &app, [0.5, 12.5], phase);
        app.drawing.virtual_events(vec![event], false);
        step(&mut app, &mut time, vec![]);
    }
    if app.documents[0].layers[n].pixel(64, 48, 0, 12) != [40, 100, 160, 255]
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != rasterized
    {
        return Err(format!(
            "Mouse paint/Undo on raster substitute failed: {:?}",
            app.status
        ));
    }
    app.execute_command(&ctx, "raster-conversion");
    {
        let e = app.raster_conversion.draft.as_mut().unwrap();
        e.operation = Operation::Color;
        e.selection = false;
        e.color = [80, 120, 160, 10];
    }
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.raster_conversion.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.raster_conversion.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.raster_conversion.draft.is_some()
        || app.documents[0].layers[n].pixel(64, 48, 0, 12) != [80, 120, 160, 128]
    {
        return Err(format!(
            "Current-color UI changed alpha or failed: {:?}",
            app.status
        ));
    }
    app.execute_command(&ctx, "raster-conversion");
    {
        let e = app.raster_conversion.draft.as_mut().unwrap();
        e.operation = Operation::Brightness;
        e.selection = false;
    }
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.raster_conversion.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.raster_conversion.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.raster_conversion.draft.is_some()
        || app.documents[0].layers[n].pixel(64, 48, 0, 12) != [0, 0, 0, 70]
    {
        return Err(format!(
            "Brightness UI independent alpha expected 70: {:?}",
            app.status
        ));
    }
    let vector = app.documents[0].scene.lines.layers[0].id;
    app.execute_command(&ctx, "raster-conversion");
    {
        let e = app.raster_conversion.draft.as_mut().unwrap();
        e.target = Key::Layer(drawer_core::layer_properties::Target::Vector(vector));
        e.operation = Operation::Color;
        e.all_paths = true;
        e.color = [30, 90, 150, 2];
    }
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.raster_conversion.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.raster_conversion.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.raster_conversion.draft.is_some()
        || app.documents[0].scene.lines.layers[0].paths[0].color != [30, 90, 150, 255]
    {
        return Err(format!("Vector content color UI failed: {:?}", app.status));
    }
    let path = root.join("raster-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
    let reopened =
        drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    if reopened.snapshot() != app.documents[0].snapshot() {
        return Err("Raster UI native roundtrip".into());
    }
    app.execute_command(&ctx, "raster-conversion");
    app.preview_raster_conversion(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_raster_conversion().is_ok() {
        return Err("Stale raster frame accepted".into());
    }
    app.documents[0].current_frame -= 1;
    let original = app.documents[0].snapshot();
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
    if app.raster_conversion.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Raster Escape changed page".into());
    }
    Ok(())
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Rasterize,
    Brightness,
    Color,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    target: Key,
    operation: Operation,
    expression: Expression,
    selection: bool,
    all_paths: bool,
    color: [u8; 4],
    prepared: Option<Box<core::Prepared>>,
    images: Option<[egui::TextureHandle; 2]>,
}
fn paths(d: &Document, key: Key, all: bool) -> Vec<u64> {
    let Key::Layer(drawer_core::layer_properties::Target::Vector(id)) = key else {
        return vec![];
    };
    if all {
        return d
            .scene
            .lines
            .layers
            .iter()
            .find(|l| l.id == id)
            .map_or(vec![], |l| l.paths.iter().map(|p| p.id).collect());
    }
    let s = &d.scene.lines.selection;
    if s.layer != Some(id) {
        return vec![];
    }
    s.paths
        .iter()
        .copied()
        .chain(s.points.iter().map(|c| c.path))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
impl Drawer {
    pub(super) fn begin_raster_conversion(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = catalog::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        )
        .ok_or("Select a layer")?;
        self.raster_conversion.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            operation: Operation::Rasterize,
            expression: Expression::Color,
            selection: true,
            all_paths: false,
            color: self.references.session.workspace.color,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_raster_conversion(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .raster_conversion
            .draft
            .as_ref()
            .ok_or("Raster conversion editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen raster conversion".into());
        }
        let action = match e.operation {
            Operation::Rasterize => Action::Rasterize(e.expression),
            Operation::Brightness => Action::Brightness {
                selection: e.selection,
            },
            Operation::Color => Action::Color {
                color: e.color,
                selection: e.selection,
                paths: paths(d, e.target, e.all_paths),
            },
        };
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(d.memory_bytes().saturating_mul(5)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Raster preview exceeds workspace memory".into());
        }
        let prepared = core::prepare(d, e.target, action)?;
        if used.saturating_add(prepared.state.memory_bytes().saturating_mul(4))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Raster result exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&prepared.state, d.current_frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "conversion-before"),
            super::layer_merge::image(ctx, &after, "conversion-after"),
        ];
        let e = self.raster_conversion.draft.as_mut().unwrap();
        e.prepared = Some(Box::new(prepared));
        e.images = Some(images);
        Ok(())
    }
    fn apply_raster_conversion(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .raster_conversion
            .draft
            .as_mut()
            .ok_or("Raster conversion editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen raster conversion".into());
        }
        let p = e.prepared.take().ok_or("Preview the conversion first")?;
        self.documents[n].commit_state(p.state)?;
        if let Some(t) = catalog::thumbnail(&self.documents[n], p.selected) {
            self.panels.targets.insert(e.id, t);
        }
        self.raster_conversion.draft = None;
        Ok(())
    }
    pub(super) fn raster_conversion_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.raster_conversion.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Rasterize and change content color").open(&mut open).default_width(550.).default_pos(egui::pos2(35.,20.)).collapsible(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen conversion.");return;}
            let mut changed=false;if let Ok(rows)=catalog::rows(d){egui::ComboBox::from_label("Layer").selected_text(rows.iter().find(|r|r.key==e.target).map_or("Missing layer",|r|r.name.as_str())).show_ui(ui,|ui|{for row in rows.into_iter().filter(|r|!matches!(r.key,Key::Selection(_)|Key::Track(_)|Key::Cell{..})){changed|=ui.selectable_value(&mut e.target,row.key,&row.name).changed();}});}
            ui.horizontal(|ui|{for(value,label)in[(Operation::Rasterize,"Rasterize"),(Operation::Brightness,"Brightness to opacity"),(Operation::Color,"Current RGB")]{changed|=ui.radio_value(&mut e.operation,value,label).changed();}});
            match e.operation{
                Operation::Rasterize=>{ui.horizontal(|ui|{ui.label("Raster expression");for v in[Expression::Color,Expression::Gray,Expression::Mono]{changed|=ui.radio_value(&mut e.expression,v,format!("{v:?}")).changed();}});ui.small("Original compositing position is retained. Complete vector clipping runs convert together. Masks and display effects become editable raster pixels. One Undo restores editable geometry.");},
                Operation::Brightness=>{changed|=ui.checkbox(&mut e.selection,"Use current area selection strength").changed();ui.small("White becomes transparent and black keeps its alpha. Gray reduces the existing alpha. Partial selection strengths mix the results.");},
                Operation::Color=>{let mut rgb=[e.color[0],e.color[1],e.color[2]];if ui.color_edit_button_srgb(&mut rgb).changed(){e.color[..3].copy_from_slice(&rgb);changed=true;}
if ui.button("Use current drawing color").clicked(){e.color=self.references.session.workspace.color;changed=true;}changed|=ui.checkbox(&mut e.selection,"Use current raster selection strength").changed();if matches!(e.target,Key::Layer(drawer_core::layer_properties::Target::Vector(_))){changed|=ui.checkbox(&mut e.all_paths,"Use all vector paths in this layer").changed();ui.label(format!("Selected vector paths: {}",paths(d,e.target,e.all_paths).len()));}ui.small("Raster alpha and selected vector stroke/fill alpha are preserved. Vector geometry and pressure outlines stay editable. Transparent pixels keep zero RGB. Correction layers and linked content need conversion first.");}
            }
            if changed{e.prepared=None;e.images=None;}
            let r=ui.button("Preview conversion");self.raster_conversion.preview=Some(r.rect);preview=r.clicked();
            if let Some(p)=&e.prepared{ui.label(format!("Affected sources: {}",p.affected.len()));for note in &p.notes{ui.small(note);}
if e.operation==Operation::Rasterize&&e.expression==Expression::Color{ui.label("Appearance comparison passed.");}}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current page",&images[0]),("After conversion",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(128.,128.)));});}});}
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some(),egui::Button::new("Apply raster conversion"));self.raster_conversion.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.raster_conversion.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_raster_conversion(ctx) {
            self.status = Some(e);
        }
        if apply && let Err(e) = self.apply_raster_conversion() {
            self.status = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn raster_ui_preview_apply_mouse_paint_color_brightness_undo_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
