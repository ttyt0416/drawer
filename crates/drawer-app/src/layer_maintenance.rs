use super::*;
use drawer_core::layer_maintenance::{self as core, Operation};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    acknowledge: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    operation: Operation,
    prepared: Option<core::Prepared>,
    images: Option<[egui::TextureHandle; 2]>,
    acknowledged: bool,
}
impl Drawer {
    pub(super) fn begin_layer_maintenance(&mut self, operation: Operation) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        self.layer_maintenance.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            operation,
            prepared: None,
            images: None,
            acknowledged: false,
        }));
        Ok(())
    }
    fn preview_layer_maintenance(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .layer_maintenance
            .draft
            .as_ref()
            .ok_or("Layer maintenance closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen the preview".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used
            .saturating_add(d.memory_bytes().saturating_mul(4))
            .saturating_add(128 * 1024 * 1024)
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Layer preview exceeds workspace memory".into());
        }
        let result = core::prepare(d, e.operation)?;
        if used
            .saturating_add(result.state.memory_bytes().saturating_mul(2))
            .saturating_add(d.memory_bytes().saturating_mul(3))
            > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Layer result exceeds workspace memory".into());
        }
        let after = drawer_core::layer_merge::preview(&result.state, e.frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "maintenance-before"),
            super::layer_merge::image(ctx, &after, "maintenance-after"),
        ];
        let e = self.layer_maintenance.draft.as_mut().unwrap();
        e.prepared = Some(result);
        e.images = Some(images);
        e.acknowledged = false;
        Ok(())
    }
    fn apply_layer_maintenance(&mut self) -> Result<(), String> {
        let n = self.active.ok_or("Open a page first")?;
        let e = self
            .layer_maintenance
            .draft
            .as_mut()
            .ok_or("Layer maintenance closed")?;
        let d = &self.documents[n];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen the preview".into());
        }
        if !e.acknowledged {
            return Err(
                "Review the result, deletion candidates and reference impacts first".into(),
            );
        }
        let result = e.prepared.take().ok_or("Prepare the layer preview first")?;
        let selected = result.selected;
        self.documents[n].commit_state(result.state)?;
        self.sync_layer_identity();
        if let Some(key) = selected
            && let Some(target) = drawer_core::layer_catalog::thumbnail(&self.documents[n], key)
        {
            self.panels.targets.insert(self.documents[n].id, target);
        }
        self.layer_maintenance.draft = None;
        Ok(())
    }
    pub(super) fn layer_maintenance_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.layer_maintenance.draft.take() else {
            return;
        };
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            return;
        };
        let d = &self.documents[n];
        let fresh = (d.revision, d.current_frame) == (e.revision, e.frame);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel, mut changed) = (false, false, false, false);
        egui::Window::new("Visible copy and layer cleanup").open(&mut open).resizable(false).collapsible(false).fixed_size(egui::vec2(580.,585.)).default_pos(egui::pos2(40.,20.)).show(ctx,|ui|{
            if !fresh {ui.label("Page/frame changed; cancel and reopen this preview.");cancel=ui.button("Cancel").clicked();return;}
            ui.horizontal_wrapped(|ui|{for op in [Operation::VisibleCopy,Operation::Empty,Operation::Hidden] {changed|=ui.radio_value(&mut e.operation,op,op.label()).changed();}});
            ui.allocate_ui(egui::vec2(560.,62.),|ui|{ui.small(match e.operation {
                Operation::VisibleCopy=>"Add the displayed page as a new raster. Originals remain visible and editable. Semi-transparent regions can become denser.",
                Operation::Empty=>"Inspect stored alpha and vector/text object counts. Corrections, references, effects, paper and protected layers are retained.",
                Operation::Hidden=>"Review hidden layer candidates, comp records and timeline exposure changes. Protected layers and necessary retained children are excluded.",
            });});
            if changed {e.prepared=None;e.images=None;e.acknowledged=false;}
            let r=ui.button("Prepare preview and candidate list");self.layer_maintenance.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(560.,190.),|ui|{egui::ScrollArea::vertical().id_salt("maintenance-results").max_height(190.).show(ui,|ui|{
                if let Some(result)=&e.prepared {
                    for note in &result.notes {ui.small(note);}
                    if e.operation!=Operation::VisibleCopy {
                        ui.strong(format!("{} deletion candidates · {} retained · {} exposure markers removed",result.candidates.len(),result.retained.len(),result.exposures));
                        for (key,name) in &result.candidates {ui.label(format!("Delete: {name} ({key:?})"));}
                        for (key,reason) in &result.retained {ui.small(format!("Keep: {reason} ({key:?})"));}
                        if result.comp_references.is_empty(){ui.label("No saved comp entries refer to these candidates.");}
                        for (name,count) in &result.comp_references {ui.colored_label(egui::Color32::YELLOW,format!("Comp ‘{name}’: {count} entries will be reported missing"));}
                    }
                }
            });});
            ui.allocate_ui(egui::vec2(560.,118.),|ui|{if let Some(images)=&e.images {ui.horizontal(|ui|{for (label,image) in ["Current page","After operation"].into_iter().zip(images) {ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}});
            let r=ui.add_enabled(e.prepared.is_some(),egui::Checkbox::new(&mut e.acknowledged,"I reviewed the result, candidates and reference impacts"));self.layer_maintenance.acknowledge=Some(r.rect);
            ui.horizontal(|ui|{let r=ui.add_enabled(e.prepared.is_some()&&e.acknowledged,egui::Button::new("Apply layer operation"));self.layer_maintenance.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
            ui.small("One Undo restores all structure and pixels. Preview and cancellation make no document changes.");
        });
        if open && !cancel {
            self.layer_maintenance.draft = Some(e);
        }
        if preview && let Err(err) = self.preview_layer_maintenance(ctx) {
            self.status = Some(err);
        }
        if apply && let Err(err) = self.apply_layer_maintenance() {
            self.status = Some(err);
        }
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_maintenance_verify::verify(root)?;
    for (command, operation) in [
        ("merge-visible-copy", Operation::VisibleCopy),
        ("delete-empty-layers", Operation::Empty),
        ("delete-hidden-layers", Operation::Hidden),
    ] {
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
        app.documents[0]
            .commit_state(drawer_core::layer_maintenance_verify::fixture()?.snapshot())?;
        let before = app.documents[0].snapshot();
        let count = app.documents[0].history_counts();
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
        app.execute_command(&ctx, command);
        if app
            .layer_maintenance
            .draft
            .as_ref()
            .is_none_or(|e| e.operation != operation)
        {
            return Err(format!("Maintenance command missing: {command}"));
        }
        for _ in 0..2 {
            step(&mut app, &mut time, vec![]);
        }
        let r = app
            .layer_maintenance
            .preview
            .ok_or("Maintenance preview button missing")?;
        click(&mut app, &mut time, r);
        if app
            .layer_maintenance
            .draft
            .as_ref()
            .is_none_or(|e| e.prepared.is_none())
        {
            return Err(format!("Maintenance preview failed: {:?}", app.status));
        }
        if app.documents[0].snapshot() != before || app.apply_layer_maintenance().is_ok() {
            return Err("Maintenance preview/confirmation bypass".into());
        }
        for _ in 0..2 {
            step(&mut app, &mut time, vec![]);
        }
        let r = app
            .layer_maintenance
            .acknowledge
            .ok_or("Maintenance confirmation missing")?;
        click(&mut app, &mut time, r);
        let r = app
            .layer_maintenance
            .apply
            .ok_or("Maintenance apply missing")?;
        click(&mut app, &mut time, r);
        if app.layer_maintenance.draft.is_some()
            || app.documents[0].history_counts().0 != count.0 + 1
        {
            return Err(format!("Maintenance apply/Undo failed: {:?}", app.status));
        }
        let after = app.documents[0].snapshot();
        if !app.documents[0].undo()
            || app.documents[0].snapshot() != before
            || !app.documents[0].redo()
            || app.documents[0].snapshot() != after
        {
            return Err("Maintenance UI Undo/Redo".into());
        }
        app.begin_layer_maintenance(Operation::VisibleCopy)?;
        app.preview_layer_maintenance(&ctx)?;
        app.layer_maintenance.draft.as_mut().unwrap().acknowledged = true;
        app.documents[0].current_frame += 1;
        if app.apply_layer_maintenance().is_ok() || app.documents[0].snapshot() != after {
            return Err("Stale maintenance frame applied".into());
        }
        app.begin_layer_maintenance(Operation::VisibleCopy)?;
        app.preview_layer_maintenance(&ctx)?;
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
        if app.layer_maintenance.draft.is_some() || app.documents[0].snapshot() != after {
            return Err("Maintenance Escape mutated document".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn visible_copy_empty_hidden_buttons_confirmation_undo_stale_escape() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
