use super::*;
use drawer_core::{
    layer_properties::{self as p, Target},
    transfer_down as core,
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
    upper: u64,
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
}
impl Drawer {
    pub(super) fn begin_transfer_down(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let t = p::from_thumbnail(
            d,
            self.panels
                .targets
                .get(&d.id)
                .copied()
                .unwrap_or(drawer_core::thumbnail::Target::Raster(0)),
        );
        let Target::Raster(upper) = t else {
            return Err("Select the upper raster layer".into());
        };
        self.transfer_down.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            upper,
            prepared: None,
            images: None,
        }));
        Ok(())
    }
    fn preview_transfer_down(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .transfer_down
            .draft
            .as_ref()
            .ok_or("Transfer preview closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen transfer".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(128 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Transfer preview exceeds workspace memory".into());
        }
        let prepared = core::prepare(d, e.upper)?;
        if used.saturating_add(prepared.state.memory_bytes().saturating_mul(4))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Transfer result exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&prepared.state, e.frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "transfer-before"),
            super::layer_merge::image(ctx, &after, "transfer-after"),
        ];
        let e = self.transfer_down.draft.as_mut().unwrap();
        e.prepared = Some(prepared);
        e.images = Some(images);
        Ok(())
    }
    fn apply_transfer_down(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .transfer_down
            .draft
            .as_mut()
            .ok_or("Transfer preview closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen transfer".into());
        }
        let prepared = e.prepared.take().ok_or("Preview transfer first")?;
        let changed = prepared.changed;
        self.documents[n].commit_state(prepared.state)?;
        self.status = Some(format!(
            "Transferred {changed} pixel positions; upper layer retained"
        ));
        self.transfer_down.draft = None;
        Ok(())
    }
    pub(super) fn transfer_down_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.transfer_down.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel) = (false, false, false);
        egui::Window::new("Transfer content to lower raster").open(&mut open).default_pos(egui::pos2(30.,20.)).default_width(480.).collapsible(false).show(ctx,|ui|{
            if !fresh{ui.label("Page/frame changed; cancel and reopen transfer.");return;}
            ui.label(format!("Upper: {}",super::layer_properties::label(d,Target::Raster(e.upper))));
            match core::lower(d,e.upper){Ok(n)=>{ui.label(format!("Lower: {}",d.layers[n].name));},Err(err)=>{ui.label(err);}}
            ui.label(if d.scene.selection.is_none()&&d.scene.selection_mask.is_none(){"Transfer all content; retain the empty upper layer."}else{"Transfer selected content only; keep pixels outside the selection. Empty selections do nothing."});
            ui.small("Both layer identities, names, order and settings stay. Soft edges preserve the combined appearance. Show and unlock both layers; use Normal/color rasters at 100% opacity without active effects, alpha protection or clipping. Rasterize effects first if needed.");
            let r=ui.button("Preview content transfer");self.transfer_down.preview=Some(r.rect);preview=r.clicked();
            if let Some(p)=&e.prepared{ui.label(format!("Pixel positions: {} / appearance difference: {}",p.changed,p.maximum_delta));}
            if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current",&images[0]),("After transfer",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some(),egui::Button::new("Transfer and retain upper layer"));self.transfer_down.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
        });
        if open && !cancel {
            self.transfer_down.draft = Some(e);
        }
        let r = if preview {
            self.preview_transfer_down(ctx)
        } else if apply {
            self.apply_transfer_down()
        } else {
            Ok(())
        };
        if let Err(e) = r {
            self.status = Some(e);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::transfer_down_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 3,
        height: 1,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::transfer_down_verify::fixture()?.snapshot())?;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(1),
    );
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
    app.execute_command(&ctx, "transfer-down");
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let r = app
        .transfer_down
        .preview
        .ok_or("Transfer preview button missing")?;
    click(&mut app, &mut time, r);
    if app.documents[0].snapshot() != original
        || app
            .transfer_down
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
    {
        return Err(format!(
            "Transfer UI preview changed source or failed: {:?}",
            app.status
        ));
    }
    step(&mut app, &mut time, vec![]);
    let r = app.transfer_down.apply.ok_or("Transfer apply missing")?;
    click(&mut app, &mut time, r);
    if app.transfer_down.draft.is_some()
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.documents[0].layers.len() != 2
        || app.documents[0].layers[1].pixels[1][3] != 64
        || app.documents[0].layers[0].pixels[1][3] != 85
    {
        return Err(format!("Transfer UI apply/one Undo: {:?}", app.status));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Transfer UI Undo failed".into());
    }
    let mut scene = app.documents[0].scene.clone();
    scene.selection_mask = None;
    app.documents[0].edit_scene(scene)?;
    app.execute_command(&ctx, "transfer-down");
    app.preview_transfer_down(&ctx)?;
    app.apply_transfer_down()?;
    if app.documents[0].layers.len() != 2
        || app.documents[0].layers[1].name != "Keep upper"
        || app.documents[0].layers[1]
            .pixels
            .iter()
            .any(|p| *p != [0; 4])
        || p::paintable(&app.documents[0], 1).is_err()
        || app.panels.targets.get(&app.documents[0].id)
            != Some(&drawer_core::thumbnail::Target::Raster(1))
    {
        return Err("Whole transfer removed the upper layer or its selection/editability".into());
    }
    app.documents[0].undo();
    app.execute_command(&ctx, "transfer-down");
    app.preview_transfer_down(&ctx)?;
    app.documents[0].current_frame += 1;
    let original = app.documents[0].snapshot();
    if app.apply_transfer_down().is_ok() || app.documents[0].snapshot() != original {
        return Err("Stale frame content transferred".into());
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
    if app.transfer_down.draft.is_some() {
        return Err("Transfer Escape failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn transfer_ui_preview_apply_retain_selection_editable_undo_stale_escape() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
