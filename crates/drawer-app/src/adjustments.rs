use super::*;
use drawer_core::{
    adjustments::{self as core, Destination},
    layer_effects::{Adjustment, ColorModel},
    layer_properties::Target,
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    edit: bool,
    target: Option<u64>,
    parent: Option<usize>,
    name: String,
    value: Adjustment,
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn begin_adjustment(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let selected = self
            .panels
            .targets
            .get(&d.id)
            .copied()
            .unwrap_or(drawer_core::thumbnail::Target::Raster(0));
        let id = if let drawer_core::thumbnail::Target::Raster(n) = selected {
            Some(d.scene.layer_ids.id(n))
        } else {
            None
        };
        let current = id.and_then(|id| {
            drawer_core::layer_effects::adjustment(d, Target::Raster(id)).map(|v| (id, v))
        });
        let target = current.map(|v| v.0).or_else(|| {
            d.scene
                .layer_effects
                .rasters
                .iter()
                .find(|(_, e)| e.adjustment.is_some())
                .map(|(id, _)| *id)
        });
        let parent = match selected {
            drawer_core::thumbnail::Target::Folder(n) => Some(n),
            drawer_core::thumbnail::Target::Raster(n) => {
                drawer_core::layers::parent(&d.scene.paper, n)
            }
            _ => None,
        };
        self.adjustments.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            edit: current.is_some(),
            target,
            parent,
            name: "Color correction".into(),
            value: current.map_or_else(Adjustment::default, |v| v.1),
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_adjustment(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .adjustments
            .draft
            .as_ref()
            .ok_or("Correction draft is closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen correction controls".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(d.memory_bytes().saturating_mul(4)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Correction preview exceeds workspace memory".into());
        }
        let dest = if e.edit {
            Destination::Existing(e.target.ok_or("Select a correction layer")?)
        } else {
            Destination::New {
                name: e.name.clone(),
                parent: e.parent,
            }
        };
        let result = core::prepare(d, dest, e.value)?;
        if used.saturating_add(result.state.memory_bytes().saturating_mul(3))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Correction result exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&result.state, d.current_frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "correction-before"),
            super::layer_merge::image(ctx, &after, "correction-after"),
        ];
        let e = self.adjustments.draft.as_mut().unwrap();
        e.prepared = Some(result);
        e.images = Some(images);
        Ok(())
    }
    fn apply_adjustment(&mut self) -> Result<(), String> {
        let e = self
            .adjustments
            .draft
            .as_mut()
            .ok_or("Correction draft is closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen correction controls".into());
        }
        let result = e.prepared.take().ok_or("Preview the correction first")?;
        let id = result.layer;
        self.documents[n].commit_state(result.state)?;
        self.sync_layer_identity();
        let d = &self.documents[n];
        if let Some(i) = drawer_core::layers::index(d, id) {
            self.panels
                .targets
                .insert(d.id, drawer_core::thumbnail::Target::Raster(i));
        }
        self.adjustments.draft = None;
        Ok(())
    }
    pub(super) fn adjustment_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.adjustments.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let existing = d
            .scene
            .layer_effects
            .rasters
            .iter()
            .filter(|(id, v)| {
                v.adjustment.is_some() && drawer_core::layers::index(d, **id).is_some()
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut cancel = false;
        let mut preview = false;
        let mut apply = false;
        let mut changed = false;
        egui::Window::new("Nondestructive color correction").open(&mut open).collapsible(false).resizable(false).default_width(540.).default_pos(egui::pos2(45.,20.)).show(ctx,|ui|{
   if !fresh{ui.label("Page/frame changed; cancel and reopen this draft.");cancel=ui.button("Cancel").clicked();return;}
   ui.horizontal(|ui|{changed|=ui.radio_value(&mut e.edit,false,"New correction layer").changed();ui.add_enabled_ui(!existing.is_empty(),|ui|{if ui.radio_value(&mut e.edit,true,"Edit correction layer").changed(){changed=true;if let Some(id)=e.target{e.value=drawer_core::layer_effects::adjustment(d,Target::Raster(id)).unwrap_or_default();}}});});
   if e.edit{egui::ComboBox::from_label("Correction layer").selected_text(e.target.map_or("Select layer".into(),|id|super::layer_properties::label(d,Target::Raster(id)))).show_ui(ui,|ui|{for id in &existing{if ui.selectable_value(&mut e.target,Some(*id),super::layer_properties::label(d,Target::Raster(*id))).changed(){changed=true;e.value=drawer_core::layer_effects::adjustment(d,Target::Raster(*id)).unwrap();}}});}else{ui.horizontal(|ui|{ui.label("Layer name");changed|=ui.text_edit_singleline(&mut e.name).changed();});egui::ComboBox::from_label("Parent folder").selected_text(e.parent.map_or("Root".into(),|n|format!("Folder {}",n+1))).show_ui(ui,|ui|{changed|=ui.selectable_value(&mut e.parent,None,"Root").changed();for(n,f)in d.scene.paper.folders.iter().enumerate().filter(|(n,_)|d.scene.paper.folder_active(*n)){changed|=ui.selectable_value(&mut e.parent,Some(n),&f.name).changed();}});}
   changed|=ui.add(egui::Slider::new(&mut e.value.brightness,-1. ..=1.).text("Brightness").custom_formatter(|n,_|format!("{:.0}%",n*100.))).changed();
   changed|=ui.add(egui::Slider::new(&mut e.value.contrast,-1. ..=1.).text("Contrast").custom_formatter(|n,_|format!("{:.0}%",n*100.))).changed();
   changed|=ui.add(egui::Slider::new(&mut e.value.hue,-180. ..=180.).text("Hue degrees")).changed();
   changed|=ui.add(egui::Slider::new(&mut e.value.saturation,-1. ..=1.).text("Saturation").custom_formatter(|n,_|format!("{:.0}%",n*100.))).changed();
   ui.horizontal(|ui|{ui.label("Saturation model");changed|=ui.radio_value(&mut e.value.color_model,ColorModel::Hsl,"HSL").changed();changed|=ui.radio_value(&mut e.value.color_model,ColorModel::Hsv,"HSV").changed();if ui.button("Reset correction").clicked(){e.value=Default::default();changed=true;}});
   ui.small("HSL preserves lightness while reducing saturation. HSV preserves value. Corrections affect the earlier raster backdrop in their folder; normal folders isolate their children.");
   if changed{e.prepared=None;e.images=None;}
   let r=ui.button("Preview correction");self.adjustments.preview=Some(r.rect);preview=r.clicked();
   if let Some(images)=&e.images{ui.horizontal(|ui|{ui.vertical(|ui|{ui.label("Current page");ui.add(egui::Image::new(&images[0]).fit_to_exact_size(egui::vec2(128.,128.)));});ui.vertical(|ui|{ui.label("After correction");ui.add(egui::Image::new(&images[1]).fit_to_exact_size(egui::vec2(128.,128.)));});});}
   ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some(),egui::Button::new("Apply correction"));self.adjustments.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
   ui.small("Original pixels and alpha stay unchanged. Layer masks and opacity limit the correction. One Undo restores parameters or the created layer.");
  });
        if open && !cancel {
            self.adjustments.draft = Some(e);
        }
        if preview && let Err(err) = self.preview_adjustment(ctx) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.apply_adjustment() {
            self.status = Some(err);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::adjustments_verify::verify(root)?;
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
    app.documents[0].layers[0].pixels.fill([255, 0, 0, 128]);
    let mut time = 0.;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let before = app.documents[0].snapshot();
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
    for edit in [false, true] {
        app.execute_command(&ctx, "adjustment-layer");
        let e = app
            .adjustments
            .draft
            .as_mut()
            .ok_or("Correction command missing")?;
        e.edit = edit;
        e.value = if edit {
            Adjustment {
                saturation: -1.,
                ..Default::default()
            }
        } else {
            Adjustment {
                hue: 120.,
                ..Default::default()
            }
        };
        let source = app.documents[0].snapshot();
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        let r = app
            .adjustments
            .preview
            .ok_or("Correction preview missing")?;
        click(&mut app, &mut time, r);
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != source {
            return Err("Correction preview changed source".into());
        }
        let r = app.adjustments.apply.ok_or("Correction Apply missing")?;
        click(&mut app, &mut time, r);
        let expected = if edit {
            [128, 128, 128, 128]
        } else {
            [0, 255, 0, 128]
        };
        if app.adjustments.draft.is_some()
            || app.documents[0].composite_pixel(0, 0) != expected
            || app.documents[0].layers[0] != before.layers[0]
        {
            return Err(format!("Actual correction edit={edit}: {:?}", app.status));
        }
        let after = app.documents[0].snapshot();
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != source
            || !app.documents[0].redo()
            || app.documents[0].snapshot() != after
        {
            return Err("Correction UI Undo/Redo".into());
        }
    }
    let file = root.join("correction-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    let saved = app.documents[0].snapshot();
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != saved
    {
        return Err("Correction UI reopen".into());
    }
    app.execute_command(&ctx, "adjustment-layer");
    app.documents[0].current_frame += 1;
    if app.preview_adjustment(&ctx).is_ok() {
        return Err("Stale correction frame accepted".into());
    }
    app.documents[0].current_frame = 0;
    app.adjustments.draft = None;
    app.execute_command(&ctx, "adjustment-layer");
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
    if app.adjustments.draft.is_some() || app.documents[0].snapshot() != saved {
        return Err("Correction Escape changed source".into());
    }
    app.execute_command(&ctx, "adjustment-layer");
    app.documents.clear();
    app.active = None;
    app.adjustment_window(&ctx);
    if app.adjustments.draft.is_some() {
        return Err("Closed correction page retained draft".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_correction_preview_create_edit_undo_and_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
