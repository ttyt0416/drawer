use super::*;
use drawer_core::{
    layer_merge::{self as core, Operation, Settings},
    layer_properties::{self as properties, Target},
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    prepare: Option<egui::Rect>,
    apply: Option<egui::Rect>,
    ack: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    selected: Vec<Target>,
    kind: u8,
    settings: Settings,
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
    acknowledged: bool,
}
pub(super) fn image(ctx: &egui::Context, d: &Document, label: &str) -> egui::TextureHandle {
    let size = [d.width.min(64) as usize, d.height.min(64) as usize];
    let r = drawer_core::render::Renderer::new(d);
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let p = r.pixel([
                (x as f64 + 0.5) * d.width as f64 / size[0] as f64,
                (y as f64 + 0.5) * d.height as f64 / size[1] as f64,
            ]);
            pixels.push(egui::Color32::from_rgba_unmultiplied(
                p[0], p[1], p[2], p[3],
            ));
        }
    }
    ctx.load_texture(
        label,
        egui::ColorImage::new(size, pixels),
        egui::TextureOptions::NEAREST,
    )
}
impl Drawer {
    pub(super) fn begin_layer_merge(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let target = properties::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        self.layer_merge.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected: vec![target],
            kind: 1,
            settings: Default::default(),
            prepared: None,
            images: None,
            acknowledged: false,
        }));
        Ok(())
    }
    fn prepare_layer_merge(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .layer_merge
            .draft
            .as_ref()
            .ok_or("Merge draft is closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen the merge draft".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(32 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Merge preview exceeds workspace memory".into());
        }
        let operation = match e.kind {
            0 => Operation::Selected(e.selected.clone()),
            1 => Operation::Down(*e.selected.first().ok_or("Choose a merge-down target")?),
            _ => Operation::Flatten,
        };
        let result = core::prepare(d, operation, e.settings)?;
        if used
            .saturating_add(d.memory_bytes().saturating_mul(3))
            .saturating_add(result.state.memory_bytes().saturating_mul(2))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Merged result exceeds workspace memory".into());
        }
        let after = core::preview(&result.state, d.current_frame)?;
        let images = [
            image(ctx, d, "merge-before"),
            image(ctx, &after, "merge-after"),
        ];
        let e = self.layer_merge.draft.as_mut().unwrap();
        e.prepared = Some(result);
        e.images = Some(images);
        e.acknowledged = false;
        Ok(())
    }
    fn apply_layer_merge(&mut self) -> Result<(), String> {
        let e = self
            .layer_merge
            .draft
            .as_mut()
            .ok_or("Merge draft is closed")?;
        let n = self.active.ok_or("Open a page first")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen the merge draft".into());
        }
        if !e.acknowledged {
            return Err("Review the rasterization preview and its affected content first".into());
        }
        let prepared = e.prepared.take().ok_or("Prepare a merge preview first")?;
        let selected = prepared.selected;
        self.documents[n].commit_state(prepared.state)?;
        self.sync_layer_identity();
        let d = &self.documents[n];
        if let Target::Raster(id) = selected
            && let Some(index) = drawer_core::layers::index(d, id)
        {
            self.panels
                .targets
                .insert(d.id, drawer_core::thumbnail::Target::Raster(index));
        }
        self.layer_merge.draft = None;
        Ok(())
    }
    pub(super) fn layer_merge_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_merge.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let targets = properties::targets(d)
            .into_iter()
            .filter(|t| matches!(t, Target::Raster(_) | Target::Folder(_)))
            .collect::<Vec<_>>();
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut prepare = false;
        let mut apply = false;
        let mut cancel = false;
        let mut changed = false;
        egui::Window::new("Merge layers and flatten page").open(&mut open).collapsible(false).resizable(false).default_width(570.).default_pos(egui::pos2(45.,20.)).show(ctx,|ui|{
   if !fresh{ui.label("Page/frame changed; cancel and reopen this draft.");cancel=ui.button("Cancel").clicked();return;}
   ui.horizontal(|ui|{changed|=ui.radio_value(&mut e.kind,0,"Merge selected").changed();changed|=ui.radio_value(&mut e.kind,1,"Merge down").changed();changed|=ui.radio_value(&mut e.kind,2,"Flatten page").changed();});
   if e.kind<2{egui::ScrollArea::vertical().id_salt("merge-targets").max_height(130.).show_rows(ui,22.,targets.len(),|ui,rows|{for n in rows{let t=targets[n];let mut selected=e.selected.contains(&t);if ui.checkbox(&mut selected,super::layer_properties::label(d,t)).changed(){changed=true;if selected{if e.kind==1{e.selected.clear();}e.selected.push(t);}else{e.selected.retain(|v|*v!=t);}}}});}
   changed|=ui.checkbox(&mut e.settings.include_hidden,"Include hidden layers (show them in the merged result)").changed();
   if e.kind<2{changed|=ui.checkbox(&mut e.settings.lower_backdrop,"Include lower siblings as the blend backdrop").changed();ui.small("Selection must be consecutive siblings. Merge down uses the first selected target.");}
   if changed{e.prepared=None;e.images=None;e.acknowledged=false;}
   let r=ui.button("Prepare merge preview");self.layer_merge.prepare=Some(r.rect);prepare=r.clicked();
   if let Some(result)=&e.prepared{ui.separator();for note in &result.notes{ui.label(note);}ui.label(format!("{} layer/folder/cell targets affected · maximum channel rounding {}",result.affected.len(),result.maximum_delta));
    if let Some(images)=&e.images{ui.horizontal(|ui|{ui.vertical(|ui|{ui.label("Current page");ui.add(egui::Image::new(&images[0]).fit_to_exact_size(egui::vec2(128.,128.)));});ui.vertical(|ui|{ui.label("After rasterization");ui.add(egui::Image::new(&images[1]).fit_to_exact_size(egui::vec2(128.,128.)));});});}
    let r=ui.checkbox(&mut e.acknowledged,"I reviewed the affected content and rasterization changes");self.layer_merge.ack=Some(r.rect);
   }
   ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some()&&e.acknowledged,egui::Button::new("Apply merge"));self.layer_merge.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
   ui.small("One Undo restores layer structure, visibility, masks and effects. Preview and cancellation leave the document untouched.");
  });
        if open && !cancel {
            self.layer_merge.draft = Some(e);
        }
        if prepare && let Err(err) = self.prepare_layer_merge(ctx) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.apply_layer_merge() {
            self.status = Some(err);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_merge_verify::verify(root)?;
    for kind in 0..3 {
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
        app.documents[0].layers[0].pixels.fill([0, 0, 255, 128]);
        let mut l = app.documents[0].layers[0].clone();
        l.pixels.fill([255, 0, 0, 128]);
        app.documents[0].layers.push(l);
        let mut time = 0.;
        let step = |app: &mut Drawer, time: &mut f64, events| {
            *time += 0.1;
            super::input_replay::frame(&ctx, app, *time, events);
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        app.execute_command(&ctx, "merge-layers");
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts().0;
        let e = app
            .layer_merge
            .draft
            .as_mut()
            .ok_or("Merge command missing")?;
        e.kind = kind;
        e.selected = if kind == 1 {
            vec![Target::Raster(2)]
        } else {
            vec![Target::Raster(1), Target::Raster(2)]
        };
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
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
        let r = app
            .layer_merge
            .prepare
            .ok_or("Merge preview button missing")?;
        click(&mut app, &mut time, r);
        step(&mut app, &mut time, vec![]);
        step(&mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != before || app.apply_layer_merge().is_ok() {
            return Err("Merge preview mutated document or bypassed review".into());
        }
        let r = app
            .layer_merge
            .ack
            .ok_or_else(|| format!("Merge acknowledgement missing: {:?}", app.status))?;
        click(&mut app, &mut time, r);
        let r = app.layer_merge.apply.ok_or("Merge Apply missing")?;
        click(&mut app, &mut time, r);
        if app.layer_merge.draft.is_some()
            || app.documents[0].layers.len() != 1
            || app.documents[0].history_counts().0 != count + 1
        {
            return Err(format!(
                "Actual merge buttons kind={kind}: {:?}",
                app.status
            ));
        }
        let merged = app.documents[0].snapshot();
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != before
            || !app.documents[0].redo()
            || app.documents[0].snapshot() != merged
        {
            return Err("Merge UI full Undo/Redo".into());
        }
        let file = root.join(format!("merge-ui-{kind}.drawer"));
        drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
        if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            != merged
        {
            return Err("Merge UI native reopen".into());
        }
        app.execute_command(&ctx, "merge-layers");
        app.documents[0].current_frame += 1;
        if app.prepare_layer_merge(&ctx).is_ok() {
            return Err("Stale merge frame accepted".into());
        }
        app.documents[0].current_frame = 0;
        app.layer_merge.draft = None;
        app.execute_command(&ctx, "merge-layers");
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
        if app.layer_merge.draft.is_some() || app.documents[0].snapshot() != merged {
            return Err("Merge Escape changed document".into());
        }
        app.execute_command(&ctx, "merge-layers");
        app.documents.clear();
        app.active = None;
        app.layer_merge_window(&ctx);
        if app.layer_merge.draft.is_some() {
            return Err("Closed merge page retained draft".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_merge_preview_review_apply_cancel_and_native() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
