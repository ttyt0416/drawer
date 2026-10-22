use super::*;
use drawer_core::{
    layer_search::Key,
    material_mapping::Tiling,
    material_transform::{self as core, Mode, Settings},
};
#[derive(Default)]
pub(super) struct Panel {
    pub(super) draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    acknowledge: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    pub(super) id: u64,
    pub(super) revision: u64,
    pub(super) frame: u32,
    pub(super) key: Key,
    pub(super) settings: Settings,
    pub(super) prepared: Option<core::Prepared>,
    pub(super) images: Option<[egui::TextureHandle; 2]>,
    pub(super) acknowledged: bool,
    pub(super) quality: crate::transform_review::Quality,
    pub(super) aspect: bool,
}
impl Drawer {
    pub(super) fn begin_material_transform(&mut self) -> Result<(), String> {
        self.finish_stroke();
        self.transform_begin_guard(3)?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let key = drawer_core::layer_catalog::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        )
        .ok_or("Select an image material, linked source or vector layer")?;
        if !core::targets(d)?.iter().any(|v| v.0 == key) {
            return Err("Select an image material, linked source or vector layer".into());
        }
        self.material_transform.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            quality: Default::default(),
            aspect: true,
            key,
            settings: core::settings(d, key)?,
            prepared: None,
            images: None,
            acknowledged: false,
        }));
        Ok(())
    }
    pub(super) fn preview_material_transform(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if let Some(e) = &mut self.material_transform.draft {
            e.prepared = None;
            e.images = None;
        }
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .material_transform
            .draft
            .as_ref()
            .ok_or("Transform window closed")?;
        if !((d.id, d.revision, d.current_frame) == (e.id, e.revision, e.frame)) {
            return Err("Page/frame/target changed; reopen the transform".into());
        }
        self.start_transform_preview(ctx, 3)
    }
    pub(super) fn apply_material_transform(&mut self) -> Result<(), String> {
        self.transform_apply_guard(3)?;
        let e = self
            .material_transform
            .draft
            .as_mut()
            .ok_or("Transform editor closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen the transform".into());
        }
        if !e.acknowledged {
            return Err("Review the transform and mask notes first".into());
        }
        let result = e
            .prepared
            .take()
            .ok_or("Prepare a transform preview first")?;
        self.documents[n].commit_state(result.state)?;
        if let Some(t) = drawer_core::layer_catalog::thumbnail(&self.documents[n], result.selected)
        {
            self.panels.targets.insert(e.id, t);
        }
        self.material_transform.draft = None;
        Ok(())
    }
    pub(super) fn material_transform_window(&mut self, ctx: &egui::Context) {
        let working = self.transform_working();
        let Some(mut e) = self.material_transform.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Image tiling and source transform").open(&mut open).fixed_size(egui::vec2(610.,595.)).default_pos(egui::pos2(40.,20.)).collapsible(false).resizable(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen the transform.");cancel=ui.button("Cancel").clicked();return;}
            let mut changed=false;let old_key=e.key;
            changed|=crate::transform_review::quality(ui,&mut e.quality);
            if working{ui.label("Calculating source transform — close or Escape to cancel");}
            let targets=core::targets(d).unwrap_or_default();let name=targets.iter().find(|v|v.0==e.key).map_or("Missing source",|v|v.1.as_str());
            egui::ComboBox::from_id_salt("material-transform-source").selected_text(name).show_ui(ui,|ui|{for (key,name) in &targets{ui.selectable_value(&mut e.key,*key,name);}});
            if old_key!=e.key{match core::settings(d,e.key){Ok(s)=>e.settings=s,Err(error)=>self.status=Some(error)}changed=true;}
            ui.horizontal(|ui|{ui.label("Transform");egui::ComboBox::from_id_salt("material-transform-mode").selected_text(e.settings.mode.label()).show_ui(ui,|ui|{for mode in Mode::ALL{changed|=ui.selectable_value(&mut e.settings.mode,mode,mode.label()).changed();}});});
            ui.allocate_ui(egui::vec2(590.,215.),|ui|{egui::ScrollArea::vertical().id_salt("material-transform-options").max_height(215.).show(ui,|ui|{
                let s=&mut e.settings;
                ui.horizontal(|ui|{ui.label("Pivot X/Y (document pixels)");for v in &mut s.pivot{changed|=ui.add(egui::DragValue::new(v).range(-1e6..=1e6)).changed();}});
                ui.horizontal(|ui|{ui.label("Move X/Y");for v in &mut s.movement{changed|=ui.add(egui::DragValue::new(v).range(-1e6..=1e6)).changed();}});
                if matches!(s.mode,Mode::ScaleRotate|Mode::Scale){changed|=crate::transform_review::scales(ui,&mut s.scale,&mut e.aspect,0.01..=100.).0;}
                if matches!(s.mode,Mode::ScaleRotate|Mode::Rotate){changed|=ui.add(egui::DragValue::new(&mut s.angle).suffix("° rotation").range(-36000. ..=36000.)).changed();}
                if s.mode==Mode::Skew{ui.horizontal(|ui|{ui.label("Horizontal / vertical skew");for v in &mut s.skew{changed|=ui.add(egui::DragValue::new(v).suffix("°").range(-85. ..=85.)).changed();}});}
                if s.mode==Mode::Free{ui.small("Free affine columns: horizontal X/Y, vertical X/Y, translation X/Y. Pivot and Move are applied with this matrix.");for (n,values) in s.free.0.chunks_mut(2).enumerate(){ui.horizontal(|ui|{ui.label(["Horizontal","Vertical","Translation"][n]);for v in values{changed|=ui.add(egui::DragValue::new(v).speed(0.01).range(-1000. ..=1000.)).changed();}});}}
                if matches!(s.mode,Mode::Distort|Mode::Perspective){ui.small("Corners are relative to the source area: (0,0) top left, (1,1) bottom right. Crossed corners are rejected.");for (n,p) in s.quad.iter_mut().enumerate(){ui.horizontal(|ui|{ui.label(["Top left","Top right","Bottom right","Bottom left"][n]);for v in p{changed|=ui.add(egui::DragValue::new(v).speed(0.01).range(-64. ..=64.)).changed();}});}}
                if s.mode==Mode::Mesh{
                    ui.horizontal(|ui|{ui.label("Columns / rows");changed|=ui.add(egui::DragValue::new(&mut s.columns).range(2..=5)).changed();changed|=ui.add(egui::DragValue::new(&mut s.rows).range(2..=5)).changed();if ui.button("Reset mesh grid").clicked(){let columns=s.columns;let rows=s.rows;s.mesh=(0..rows).flat_map(|y|(0..columns).map(move|x|[x as f64/(columns-1)as f64,y as f64/(rows-1)as f64])).collect();changed=true;}});
                    for (n,p) in s.mesh.iter_mut().enumerate(){ui.horizontal(|ui|{ui.label(format!("Point {}",n+1));for v in p{changed|=ui.add(egui::DragValue::new(v).speed(0.01).range(-64. ..=64.)).changed();}});}
                }
                let vector=matches!(e.key,Key::Layer(drawer_core::layer_properties::Target::Vector(_)))&&d.scene.file_objects.get(e.key).is_none();
                changed|=ui.add_enabled(vector,egui::Checkbox::new(&mut s.preserve_width,"Preserve editable vector stroke width")).changed();
                ui.add_enabled_ui(!vector,|ui|{
                    ui.horizontal(|ui|{ui.label("Tile axes");for (axes,label) in [([false;2],"None"),([true;2],"Vertical and horizontal"),([true,false],"Only horizontal"),([false,true],"Only vertical")]{changed|=ui.selectable_value(&mut s.repeat,axes,label).changed();}});
                    ui.horizontal(|ui|{for tiling in Tiling::ALL{changed|=ui.radio_value(&mut s.tiling,tiling,tiling.label()).changed();}});
                    ui.small("Reverse mirrors alternate tiles per axis. Flip rotates checkerboard-alternate tiles 180°. Original RGBA pixels remain unchanged.");
                });
            });});
            if changed{e.prepared=None;e.images=None;e.acknowledged=false;}
            let r=ui.button("Prepare source transform preview");self.material_transform.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(590.,65.),|ui|{egui::ScrollArea::vertical().id_salt("transform-notes").max_height(65.).show(ui,|ui|{if let Some(p)=&e.prepared{for note in &p.notes{ui.small(note);}}});});
            ui.allocate_ui(egui::vec2(590.,118.),|ui|{if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in["Current page","After transform"].into_iter().zip(images){ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}});
            let r=ui.add_enabled(e.prepared.is_some(),egui::Checkbox::new(&mut e.acknowledged,"I reviewed the source transform and mask notes"));self.material_transform.acknowledge=Some(r.rect);
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some()&&e.acknowledged,egui::Button::new("Apply source transform"));self.material_transform.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.material_transform.draft = Some(e);
        }
        if preview && let Err(error) = self.preview_material_transform(ctx) {
            self.status = Some(error);
        }
        if apply && let Err(error) = self.apply_material_transform() {
            self.status = Some(error);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::material_transform_verify::verify(root)?;
    for mode in Mode::ALL {
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
        let (d, key) = drawer_core::material_transform_verify::material()?;
        app.documents[0].commit_state(d.snapshot())?;
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Source(key),
        );
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            super::input_replay::frame(&ctx, app, *time, events);
            app.wait_transform_preview(&ctx, time)
                .expect("Transform preview timeout");
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
        app.execute_command(&ctx, "material-transform");
        app.material_transform
            .draft
            .as_mut()
            .ok_or("Transform command missing")?
            .settings = drawer_core::material_transform_verify::options(mode);
        for _ in 0..2 {
            step(&mut app, &mut time, vec![]);
        }
        let r = app
            .material_transform
            .preview
            .ok_or("Transform preview missing")?;
        click(&mut app, &mut time, r);
        if app
            .material_transform
            .draft
            .as_ref()
            .unwrap()
            .prepared
            .is_none()
            || app.documents[0].snapshot() != before
        {
            return Err(format!("Transform preview failed: {:?}", app.status));
        }
        if app.apply_material_transform().is_ok() {
            return Err("Transform confirmation bypassed".into());
        }
        for _ in 0..2 {
            step(&mut app, &mut time, vec![]);
        }
        let r = app.material_transform.acknowledge.unwrap();
        click(&mut app, &mut time, r);
        let r = app.material_transform.apply.unwrap();
        click(&mut app, &mut time, r);
        if app.material_transform.draft.is_some()
            || app.documents[0].history_counts().0 != count.0 + 1
        {
            return Err(format!("Transform apply failed: {:?}", app.status));
        }
        let after = app.documents[0].snapshot();
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != before
            || !app.documents[0].redo()
            || app.documents[0].snapshot() != after
        {
            return Err("Transform UI Undo/Redo".into());
        }
        app.begin_material_transform()?;
        app.preview_material_transform(&ctx)?;
        app.wait_transform_preview(&ctx, &mut time)?;
        app.material_transform.draft.as_mut().unwrap().acknowledged = true;
        app.documents[0].current_frame += 1;
        if app.apply_material_transform().is_ok() || app.documents[0].snapshot() != after {
            return Err("Stale transform applied".into());
        }
        app.begin_material_transform()?;
        app.preview_material_transform(&ctx)?;
        app.wait_transform_preview(&ctx, &mut time)?;
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
        if app.material_transform.draft.is_some() || app.documents[0].snapshot() != after {
            return Err("Transform Escape mutated page".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_transform_all_mode_buttons_confirmation_undo_stale_escape() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
