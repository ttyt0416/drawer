use super::*;
use drawer_core::{
    appearance::{self as core, Style},
    layer_properties::Target,
    layer_search::Expression,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    bake: Option<egui::Rect>,
    convert: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    target: Target,
    style: Style,
    expression: Expression,
    thresholds: drawer_core::expression_color::Settings,
    convert: bool,
    state: Option<drawer_core::sizing::DocumentState>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn begin_appearance(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = drawer_core::layer_properties::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        if !matches!(target, Target::Raster(_) | Target::Vector(_))
            || drawer_core::layer_effects::adjustment(d, target).is_some()
        {
            return Err("Choose a raster or vector layer; folder paper and corrections have their own editors".into());
        }
        self.appearance.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            style: d.scene.appearance.get(target).cloned().unwrap_or_default(),
            expression: core::expression(d, target),
            thresholds: d
                .scene
                .appearance
                .get(target)
                .and_then(|s| s.expression_settings)
                .unwrap_or_default(),
            convert: false,
            state: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_appearance(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .appearance
            .draft
            .as_ref()
            .ok_or("Display recipe editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen display effects".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(d.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Display preview exceeds workspace memory".into());
        }
        let state = if e.convert {
            drawer_core::expression_color::prepare_pixels(
                d,
                e.target,
                e.style.clone(),
                e.expression,
                e.thresholds,
            )?
        } else {
            drawer_core::expression_color::prepare_display(
                d,
                e.target,
                e.style.clone(),
                e.expression,
                e.thresholds,
            )?
        };
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "appearance-before"),
            super::layer_merge::image(ctx, &after, "appearance-after"),
        ];
        let e = self.appearance.draft.as_mut().unwrap();
        e.state = Some(state);
        e.images = Some(images);
        Ok(())
    }
    fn apply_appearance(&mut self, bake: bool) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .appearance
            .draft
            .as_mut()
            .ok_or("Display recipe editor closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen display effects".into());
        }
        let mut state = e.state.take().ok_or("Preview the display recipe first")?;
        if bake {
            let Target::Raster(id) = e.target else {
                return Err(
                    "Use Flatten page to rasterize vectors and their display effects".into(),
                );
            };
            let index = drawer_core::layers::index(d, id).ok_or("Missing raster")?;
            let mut preview = drawer_core::layer_merge::preview(&state, d.current_frame)?;
            drawer_core::paper::bake(&mut preview, drawer_core::paper::Node::Raster(index))?;
            state = preview.snapshot();
        }
        self.documents[n].commit_state(state)?;
        self.appearance.draft = None;
        Ok(())
    }
    pub(super) fn appearance_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.appearance.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut cancel = false;
        let mut preview = false;
        let mut apply = None;
        egui::Window::new("Layer display effects").open(&mut open).default_width(530.).default_pos(egui::pos2(35.,20.)).collapsible(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen display effects.");return;}
            let mut changed=false;
            egui::ScrollArea::vertical().id_salt("appearance-controls").max_height(310.).show(ui, |ui| {
            let r = ui.checkbox(&mut e.convert, "Convert stored ink (lossy; preview before applying)");
            self.appearance.convert = Some(r.rect); changed |= r.changed();
            ui.label(super::layer_properties::label(d,e.target));changed|=ui.checkbox(&mut e.style.enabled,"Enable display effects").changed();
            let mut use_color=e.style.color.is_some();if ui.checkbox(&mut use_color,"Display color").changed(){changed=true;e.style.color=use_color.then_some([self.references.session.workspace.color[0],self.references.session.workspace.color[1],self.references.session.workspace.color[2]]);}
            if let Some(c)=e.style.color.as_mut(){changed|=ui.color_edit_button_srgb(c).changed();}
            changed|=ui.add(egui::Slider::new(&mut e.style.border,0..=32).text("Square border width (document pixels)")).changed();
            ui.horizontal(|ui|{ui.label("Border color");let mut c=[e.style.border_color[0],e.style.border_color[1],e.style.border_color[2]];if ui.color_edit_button_srgb(&mut c).changed(){e.style.border_color[..3].copy_from_slice(&c);changed=true;}changed|=ui.add(egui::Slider::new(&mut e.style.border_color[3],0..=255).text("Border alpha")).changed();});
            changed|=ui.add(egui::Slider::new(&mut e.style.rim,0. ..=1.).text("Watercolor edge strength")).changed();
            ui.horizontal(|ui|{ui.label("Expression color");for value in [Expression::Color,Expression::Gray,Expression::Mono]{changed|=ui.radio_value(&mut e.expression,value,format!("{value:?}")).changed();}});
            ui.horizontal(|ui|{if ui.button("Use current area selection").clicked(){match core::capture_scope(d){Ok(s)=>{e.style.scope=Some(s);changed=true;},Err(err)=>self.status=Some(err)}}
if ui.button("Use whole layer").clicked(){e.style.scope=None;changed=true;}ui.label(if e.style.scope.is_some(){"Frozen area selection"}else{"Whole layer"});});
            if ui.button("Reset display effects").clicked(){e.style=Style::default();changed=true;}
            ui.add_enabled_ui(e.expression == Expression::Mono, |ui| {
                changed |= ui.add(egui::Slider::new(&mut e.thresholds.brightness, 0..=255).text("Monochrome brightness threshold (>= white)")).changed();
                changed |= ui.add(egui::Slider::new(&mut e.thresholds.alpha, 0..=255).text("Monochrome alpha threshold (>= opaque)")).changed();
            });
            ui.small("Gray: linear sRGB luminance, encoded to sRGB; alpha unchanged. Mono: brightness >= threshold is white, alpha >= threshold is opaque. Zero alpha stays transparent. Vector conversion changes stroke/fill colors; live thresholds apply after coverage.");
            });
            if changed{e.state=None;e.images=None;}
            let r=ui.button("Preview display effects");self.appearance.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(300.,155.), |ui| {
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in [("Current page",&images[0]),("After display effects",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(128.,128.)));});}});}
            });
            ui.horizontal(|ui|{let r=ui.add_enabled(e.state.is_some(),egui::Button::new(if e.convert {"Convert stored ink"}else{"Apply live effects"}));self.appearance.apply=Some(r.rect);if r.clicked(){apply=Some(false);}let r=ui.add_enabled(!e.convert&&e.state.is_some()&&matches!(e.target,Target::Raster(_)),egui::Button::new("Bake into raster pixels"));self.appearance.bake=Some(r.rect);if r.clicked(){apply=Some(true);}cancel=ui.button("Cancel").clicked();});
            ui.small("Live effects preserve original RGBA and paths. Turning effects off restores the source appearance. Expression color is a separate nondestructive setting. Baking replaces the raster with rendered pixels; each action uses one Undo. Folder paper uses its own editor; Flatten page converts vector effects.");
        });
        if open && !cancel {
            self.appearance.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_appearance(ctx) {
            self.status = Some(e);
        }
        if let Some(bake) = apply
            && let Err(e) = self.apply_appearance(bake)
        {
            self.status = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::appearance_verify::verify(root)?;
    drawer_core::expression_color_verify::verify(root)?;
    thresholds_smoke()?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels[3 * 8 + 3] = [200, 100, 50, 128];
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
    let before = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    app.execute_command(&ctx, "layer-appearance");
    app.appearance
        .draft
        .as_mut()
        .ok_or("Display command missing")?
        .style = Style {
        enabled: true,
        color: Some([20, 40, 60]),
        border: 1,
        rim: 0.5,
        ..Default::default()
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .appearance
        .preview
        .ok_or("Display preview button missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != counts {
        return Err("Display preview changed source/history".into());
    }
    let r = app.appearance.apply.ok_or("Display Apply missing")?;
    click(&mut app, &mut time, r);
    if app.appearance.draft.is_some()
        || drawer_core::render::Renderer::new(&app.documents[0]).pixel([3.5; 2])
            != [10, 20, 30, 128]
        || app.documents[0].layers[0].pixels != before.layers[0].pixels
        || app.documents[0].history_counts().0 != counts.0 + 1
    {
        return Err(format!("Display Apply failed: {:?}", app.status));
    }
    let live = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != live
    {
        return Err("Display Apply Undo/Redo".into());
    }
    app.execute_command(&ctx, "layer-appearance");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.appearance.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .appearance
        .bake
        .ok_or("Destructive display bake missing")?;
    click(&mut app, &mut time, r);
    if !app.documents[0].scene.appearance.is_default()
        || app.documents[0].layers[0].pixel(8, 8, 2, 3) != [0, 0, 0, 128]
    {
        return Err(format!("Display bake button: {:?}", app.status));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != live {
        return Err("Destructive display bake Undo".into());
    }
    let file = root.join("appearance-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != live
    {
        return Err("Display UI native reopen".into());
    }
    app.execute_command(&ctx, "layer-appearance");
    app.documents[0].current_frame = 1;
    if app.preview_appearance(&ctx).is_ok() {
        return Err("Stale display frame accepted".into());
    }
    app.documents[0].current_frame = 0;
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
    if app.appearance.draft.is_some() || app.documents[0].snapshot() != live {
        return Err("Display Escape changed source".into());
    }
    Ok(())
}
fn thresholds_smoke() -> Result<(), String> {
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 4,
        height: 1,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].layers[0].pixels =
        vec![[127, 127, 127, 127], [128; 4], [129; 4], [200, 10, 30, 0]];
    let original = app.documents[0].snapshot();
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
    for _ in 0..2 {
        step(&mut app, &mut time, vec![]);
    }
    for convert in [false, true] {
        let counts = app.documents[0].history_counts();
        app.execute_command(&ctx, "layer-appearance");
        let e = app
            .appearance
            .draft
            .as_mut()
            .ok_or("Expression editor missing")?;
        e.expression = Expression::Mono;
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if convert {
            let r = app
                .appearance
                .convert
                .ok_or("Expression conversion toggle missing")?;
            click(&mut app, &mut time, r);
            for _ in 0..2 {
                step(&mut app, &mut time, vec![]);
            }
            if !app.appearance.draft.as_ref().unwrap().convert {
                return Err("Expression conversion checkbox did not switch preview mode".into());
            }
        }
        let r = app.appearance.preview.ok_or("Expression preview missing")?;
        click(&mut app, &mut time, r);
        for _ in 0..3 {
            step(&mut app, &mut time, vec![]);
        }
        if app
            .appearance
            .draft
            .as_ref()
            .is_none_or(|e| e.state.is_none())
            || app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
        {
            return Err(format!(
                "Expression preview changed source/history: {:?}",
                app.status
            ));
        }
        let r = app.appearance.apply.ok_or("Expression Apply missing")?;
        click(&mut app, &mut time, r);
        let d = &app.documents[0];
        let expected = vec![[0; 4], [255; 4], [255; 4], [0; 4]];
        if app.appearance.draft.is_some()
            || drawer_core::render::Renderer::new(d).output(300., 1024)?.2 != expected
            || d.history_counts().0 != counts.0 + 1
            || (if convert {
                d.layers[0].pixels != expected
            } else {
                d.layers != original.layers
            })
        {
            return Err(format!(
                "Expression live/actual Apply failed ({convert}): {:?}",
                app.status
            ));
        }
        let applied = app.documents[0].snapshot();
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != original
            || !app.documents[0].redo()
            || app.documents[0].snapshot() != applied
            || !app.documents[0].undo()
        {
            return Err("Expression live/actual Undo/Redo".into());
        }
        if convert {
            break;
        }
    }
    app.execute_command(&ctx, "layer-appearance");
    app.appearance.draft.as_mut().unwrap().expression = Expression::Gray;
    app.preview_appearance(&ctx)?;
    app.documents[0].current_frame = 1;
    if app.apply_appearance(false).is_ok() {
        return Err("Stale expression preview accepted".into());
    }
    app.documents[0].current_frame = 0;
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
    if app.appearance.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Expression preview Escape changed source".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn real_display_preview_live_apply_bake_undo_and_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
