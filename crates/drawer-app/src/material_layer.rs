use super::*;
use drawer_core::{
    material_layer::{self as core, Layer, Source},
    scene::{Affine, PageObject, Rect},
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    load: Option<egui::Rect>,
    metal: Option<egui::Rect>,
    fallback: Option<egui::Rect>,
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::material_layer_verify::verify(root)?;
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
    app.execute_command(&ctx, "material-layer-new");
    app.material_layer
        .draft
        .as_mut()
        .ok_or("Material command missing")?
        .layer
        .source = Source::Solid([20, 40, 60, 128]);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app
        .material_layer
        .preview
        .ok_or("Material preview button missing")?;
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    if app
        .material_layer
        .draft
        .as_ref()
        .is_none_or(|e| e.state.is_none())
        || app.documents[0].snapshot() != original
    {
        return Err(format!(
            "Material preview mutated page or failed: {:?}",
            app.status
        ));
    }
    let r = app.material_layer.apply.ok_or("Material apply missing")?;
    click(&mut app, &mut time, r);
    if app.material_layer.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || drawer_core::render::Renderer::new(&app.documents[0]).pixel([2.5; 2])
            != [20, 40, 60, 128]
    {
        return Err(format!("Material apply/one Undo failed: {:?}", app.status));
    }
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != original
        || !app.documents[0].redo()
    {
        return Err("Material UI Undo/Redo".into());
    }
    let png = root.join("local-material.png");
    let mut source = Document::create(
        CanvasSpec {
            width: 2,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    source.layers[0].pixels = vec![[210, 40, 20, 128], [0; 4]];
    drawer_core::png_export::export(&source, &png, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    app.execute_command(&ctx, "material-layer-edit");
    app.material_layer
        .draft
        .as_mut()
        .ok_or("Material edit missing")?
        .path = png.to_string_lossy().into_owned();
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.material_layer.load.ok_or("Material load missing")?;
    click(&mut app, &mut time, r);
    if !matches!(&app.material_layer.draft.as_ref().ok_or("Material editor lost")?.layer.source,Source::Image(i)if i.width==2&&i.height==1)
    {
        return Err(format!(
            "Material local image button failed: {:?}",
            app.status
        ));
    }
    app.material_layer.draft.as_mut().unwrap().layer.repeat = [true; 2];
    step(&mut app, &mut time, vec![]);
    let r = app.material_layer.preview.unwrap();
    click(&mut app, &mut time, r);
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let r = app.material_layer.apply.unwrap();
    click(&mut app, &mut time, r);
    if app.material_layer.draft.is_some()
        || drawer_core::render::Renderer::new(&app.documents[0]).pixel([6.5, 5.5])
            != [210, 40, 20, 128]
    {
        return Err(format!(
            "Material image edit/apply failed: {:?}",
            app.status
        ));
    }
    let path = root.join("material-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
    std::fs::remove_file(png).map_err(|e| e.to_string())?;
    let reopened =
        drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    if reopened.snapshot() != app.documents[0].snapshot() {
        return Err("Material needed original image after native reopen".into());
    }
    app.execute_command(&ctx, "material-layer-edit");
    app.preview_material_layer(&ctx)?;
    app.documents[0].current_frame += 1;
    if app.apply_material_layer().is_ok() {
        return Err("Stale material frame accepted".into());
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
    if app.material_layer.draft.is_some() || app.documents[0].snapshot() != original {
        return Err("Material Escape changed source".into());
    }
    Ok(())
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    target: Option<u64>,
    name: String,
    path: String,
    layer: Layer,
    movement: [f64; 2],
    scale: [f64; 2],
    angle: f64,
    state: Option<drawer_core::sizing::DocumentState>,
    selected: u64,
    image: Option<egui::TextureHandle>,
}
impl Drawer {
    pub(super) fn begin_material_layer(&mut self, edit: bool) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let (target, name, layer) = if edit {
            let key = drawer_core::layer_catalog::from_thumbnail(
                d,
                self.panels
                    .targets
                    .get(&d.id)
                    .copied()
                    .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
            )
            .ok_or("Select a material layer")?;
            if d.scene.file_objects.get(key).is_some() {
                return Err("Use Linked file layer to refresh or transform this source".into());
            }
            let drawer_core::layer_search::Key::Object(id) = key else {
                return Err("Select a material layer".into());
            };
            let n = d
                .scene
                .catalog
                .objects
                .index(id, d.scene.objects.len())
                .ok_or("Missing material")?;
            let PageObject::Material(m) = &d.scene.objects[n] else {
                return Err("Select a material layer".into());
            };
            (
                Some(id),
                d.scene
                    .catalog
                    .object_info
                    .get(&id)
                    .and_then(|i| i.name.clone())
                    .unwrap_or("Material".into()),
                m.as_ref().clone(),
            )
        } else {
            (
                None,
                "Material".into(),
                Layer {
                    source: Source::Solid(self.references.session.workspace.color),
                    area: Rect {
                        min: [0.; 2],
                        max: [d.width as f64, d.height as f64],
                    },
                    transform: d.scene.transform.inverse(),
                    repeat: [false; 2],
                    mapping: Default::default(),
                },
            )
        };
        self.material_layer.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            target,
            name,
            path: String::new(),
            layer,
            movement: [0.; 2],
            scale: [1.; 2],
            angle: 0.,
            state: None,
            selected: 0,
            image: None,
        }));
        Ok(())
    }
    pub(super) fn begin_metal_material(&mut self) -> Result<(), String> {
        self.begin_material_layer(false)?;
        let e = self.material_layer.draft.as_mut().unwrap();
        e.name = "Metallic foil".into();
        e.layer.source = Source::Metal(Box::default());
        Ok(())
    }
    fn preview_material_layer(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .material_layer
            .draft
            .as_ref()
            .ok_or("Material editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen materials".into());
        }
        let mut layer = e.layer.clone();
        let (s, c) = e.angle.to_radians().sin_cos();
        let delta = Affine([
            c * e.scale[0],
            s * e.scale[0],
            -s * e.scale[1],
            c * e.scale[1],
            e.movement[0],
            e.movement[1],
        ]);
        layer.transform = layer
            .transform
            .then(d.scene.transform)
            .then(delta)
            .then(d.scene.transform.inverse());
        let (mut state, selected) = core::prepare(d, e.target, &e.name, layer)?;
        if e.target.is_none() {
            super::layer_defaults::apply(
                d,
                &mut state,
                &self.workspaces.store.current.layer_defaults,
                self.documents.iter().map(Document::resident_bytes).sum(),
            )?;
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(state.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Material preview exceeds workspace memory".into());
        }
        let preview = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let image = super::layer_merge::image(ctx, &preview, "material-preview");
        let e = self.material_layer.draft.as_mut().unwrap();
        e.state = Some(state);
        e.selected = selected;
        e.image = Some(image);
        Ok(())
    }
    fn apply_material_layer(&mut self) -> Result<(), String> {
        let e = self
            .material_layer
            .draft
            .as_mut()
            .ok_or("Material editor closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen materials".into());
        }
        self.documents[n].commit_state(e.state.take().ok_or("Preview the material first")?)?;
        self.panels.targets.insert(
            e.id,
            drawer_core::thumbnail::Target::Source(drawer_core::layer_search::Key::Object(
                e.selected,
            )),
        );
        self.material_layer.draft = None;
        Ok(())
    }
    pub(super) fn material_layer_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.material_layer.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let fresh =
            (self.documents[n].revision, self.documents[n].current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Editable material layer").open(&mut open).default_width(510.).default_pos(egui::pos2(35.,20.)).collapsible(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen materials.");return;}
            let mut changed=ui.text_edit_singleline(&mut e.name).changed();
            ui.horizontal(|ui|{if ui.button("Solid color").clicked(){e.layer.source=Source::Solid(self.references.session.workspace.color);changed=true;}
if ui.button("Gradient").clicked(){e.layer.source=Source::Gradient(Box::default());changed=true;}
let r=ui.button("Metallic foil");self.material_layer.metal=Some(r.rect);if r.clicked(){let texture=e.layer.source.image().cloned();e.layer.source=Source::Metal(Box::new(drawer_core::metal_material::Metal{texture,..Default::default()}));changed=true;}});
            match &mut e.layer.source{
                Source::Solid(c)=>{ui.label("Fill RGBA");changed|=ui.color_edit_button_srgba_unmultiplied(c).changed();},
                Source::Gradient(g)=>{
                    ui.horizontal(|ui|{changed|=ui.color_edit_button_srgba_unmultiplied(&mut g.first).changed();changed|=ui.color_edit_button_srgba_unmultiplied(&mut g.last).changed();});
                    egui::ComboBox::from_label("Gradient shape").selected_text(format!("{:?}",g.shape)).show_ui(ui,|ui|{for v in drawer_core::gradient::Shape::ALL.into_iter().filter(|s|*s!=drawer_core::gradient::Shape::Freeform){changed|=ui.selectable_value(&mut g.shape,v,format!("{v:?}")).changed();}});
                    egui::ComboBox::from_label("Gradient repeat").selected_text(format!("{:?}",g.repeat)).show_ui(ui,|ui|{for v in drawer_core::gradient::Repeat::ALL{changed|=ui.selectable_value(&mut g.repeat,v,format!("{v:?}")).changed();}});
                    ui.horizontal(|ui|{ui.label("Start X/Y");for v in &mut g.start{changed|=ui.add(egui::DragValue::new(v).range(-1e6..=1e6)).changed();}ui.label("End X/Y");for v in &mut g.end{changed|=ui.add(egui::DragValue::new(v).range(-1e6..=1e6)).changed();}});
                    changed|=ui.add(egui::Slider::new(&mut g.ellipse_ratio,0.01..=100.).logarithmic(true).text("Ellipse ratio")).changed();changed|=ui.checkbox(&mut g.dithering,"Dither gradient").changed();
                },
                Source::Image(i)=>{ui.label(format!("Embedded image: {} × {} pixels",i.width,i.height));},
                Source::Metal(m)=>{
                    ui.label("Metal tint (sRGB)");changed|=ui.color_edit_button_srgb(&mut m.color).changed();
                    changed|=ui.add(egui::Slider::new(&mut m.strength,0. ..=1.).text("Metal strength")).changed();
                    changed|=ui.add(egui::Slider::new(&mut m.gloss,0. ..=1.).text("Gloss")).changed();
                    changed|=ui.add(egui::Slider::new(&mut m.direction,0. ..=360.).text("Light direction").suffix("°")).changed();
                    if m.fallback(){ui.label("No texture: using the built-in procedural foil pattern.");}else{let i=m.texture.as_ref().unwrap();ui.label(format!("Embedded user texture: {} × {}",i.width,i.height));}
                    let r=ui.button("Use procedural fallback");self.material_layer.fallback=Some(r.rect);if r.clicked(){m.texture=None;changed=true;}
                    ui.small("Texture luminance controls relief; alpha stays intact. Strength 0 shows the unmodified user texture, or the tint when no texture is provided.");
                }
            }
            ui.horizontal(|ui|{ui.label("Local image path");ui.text_edit_singleline(&mut e.path);let r=ui.button("Load image");self.material_layer.load=Some(r.rect);if r.clicked(){match core::load(std::path::Path::new(&e.path)){Ok(source)=>{if let Source::Metal(m)=&mut e.layer.source {m.texture=source.image().cloned();}else{e.layer.source=source;}changed=true;},Err(err)=>{if let Source::Metal(m)=&mut e.layer.source{m.texture=None;changed=true;self.status=Some(format!("Texture unavailable; using procedural foil: {err}"));}else{self.status=Some(err);}}}}});
            ui.horizontal(|ui|{changed|=ui.checkbox(&mut e.layer.repeat[0],"Repeat image horizontally").changed();changed|=ui.checkbox(&mut e.layer.repeat[1],"Repeat image vertically").changed();});
            ui.horizontal(|ui|{ui.label("Local area left/top/right/bottom");for v in e.layer.area.min.iter_mut().chain(&mut e.layer.area.max){changed|=ui.add(egui::DragValue::new(v).range(-1e6..=1e6)).changed();}});
            ui.horizontal(|ui|{ui.label("Move X/Y");for v in &mut e.movement{changed|=ui.add(egui::DragValue::new(v).range(-1e6..=1e6)).changed();}});
            ui.horizontal(|ui|{ui.label("Scale X/Y");for v in &mut e.scale{changed|=ui.add(egui::DragValue::new(v).speed(0.01).range(-1000. ..=1000.)).changed();}changed|=ui.add(egui::DragValue::new(&mut e.angle).suffix("° rotation").range(-360. ..=360.)).changed();});
            if changed{e.state=None;e.image=None;}
            let r=ui.button("Preview material");self.material_layer.preview=Some(r.rect);preview=r.clicked();
            if let Some(image)=&e.image{ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(128.,128.)));}
            ui.horizontal(|ui|{let r=ui.add_enabled(e.state.is_some(),egui::Button::new("Apply material"));self.material_layer.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
            ui.small("Images keep their original pixels and transparency. Repetition uses the image's native dimensions inside the local area. Move, scale and rotate change the material, and each Apply uses one Undo. Saved pages embed the image.");
        });
        if open && !cancel {
            self.material_layer.draft = Some(e);
        }
        if preview && let Err(e) = self.preview_material_layer(ctx) {
            self.status = Some(e);
        }
        if apply && let Err(e) = self.apply_material_layer() {
            self.status = Some(e);
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn material_ui_create_preview_edit_image_apply_undo_reopen_cancel() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}

#[path = "metal_material_verify.rs"]
mod metal_verify;
pub(super) fn metal_smoke(root: &std::path::Path) -> Result<(), String> {
    metal_verify::smoke(root)
}
