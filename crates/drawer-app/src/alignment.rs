use super::*;
use drawer_core::alignment::{self as core, Align, Item, Operation, Reference};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    apply: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    selected: Vec<Item>,
    reference: Reference,
}
impl Drawer {
    pub(super) fn begin_alignment(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        self.alignment.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected: vec![],
            reference: Reference::Selection,
        }));
        Ok(())
    }
    fn apply_alignment(&mut self, op: Operation) -> Result<(), String> {
        let e = self
            .alignment
            .draft
            .as_mut()
            .ok_or("Alignment editor is closed")?;
        let used = self
            .documents
            .iter()
            .map(Document::resident_bytes)
            .sum::<usize>();
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        if (d.id, d.revision, d.current_frame) != (e.id, e.revision, e.frame) {
            return Err("Page/frame changed; reopen alignment".into());
        }
        let state = core::prepare(d, &e.selected, e.reference, op)?;
        if state.memory_bytes().saturating_mul(2).saturating_add(used) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Alignment exceeds workspace memory".into());
        }
        d.commit_state(state)?;
        e.revision = d.revision;
        Ok(())
    }
    pub(super) fn alignment_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.alignment.draft.as_mut() else {
            return;
        };
        let Some(n) = self.active else { return };
        let d = &self.documents[n];
        let items = core::items(d);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut operation = None;
        self.alignment.apply = None;
        egui::Window::new("Align and distribute objects").open(&mut open).collapsible(false).default_width(440.0).show(ctx,|ui|{
            ui.horizontal(|ui|{ui.radio_value(&mut e.reference,Reference::Selection,"Selection bounds");ui.radio_value(&mut e.reference,Reference::Canvas,"Canvas");});
            egui::ComboBox::from_id_salt("alignment-key").selected_text(match e.reference{Reference::Key(i)=>format!("Key: {}",label(d,i)),_=>"Choose reference object".into()}).show_ui(ui,|ui|{for i in &items{ui.selectable_value(&mut e.reference,Reference::Key(*i),label(d,*i));}});
            ui.horizontal(|ui|{if ui.button("Select all").clicked(){e.selected=items.clone();}
if ui.button("Select none").clicked(){e.selected.clear();}});
            egui::ScrollArea::vertical().max_height(200.0).show(ui,|ui|{for item in &items{let mut selected=e.selected.contains(item);if ui.checkbox(&mut selected,label(d,*item)).changed(){if selected{e.selected.push(*item)}else{e.selected.retain(|i|i!=item)}}}});
            ui.horizontal_wrapped(|ui|{for a in Align::ALL{let r=ui.button(format!("{a:?}"));if a==Align::Left{self.alignment.apply=Some(r.rect);}
if r.clicked(){operation=Some(Operation::Align(a));}}});
            ui.horizontal_wrapped(|ui|{for (label,op) in [("Equal horizontal gaps",Operation::GapsHorizontal),("Equal vertical gaps",Operation::GapsVertical),("Equal horizontal centers",Operation::CentersHorizontal),("Equal vertical centers",Operation::CentersVertical)]{if ui.button(label).clicked(){operation=Some(op);}}});
            ui.label("Each click applies one Undo to all selected objects. Selection bounds uses the page selection when present; otherwise the selected objects' outer bounds. A key object stays fixed. Distribution needs at least three moving objects. Raster moves round to whole pixels. Locked/hidden targets or insufficient spacing reject the whole operation.");
        });
        if !open {
            self.alignment.draft = None;
            return;
        }
        if let Some(op) = operation
            && let Err(error) = self.apply_alignment(op)
        {
            self.status = Some(error);
        }
    }
}
fn label(d: &Document, i: Item) -> String {
    match i {
        Item::Raster(n) => format!("Raster {} · {}", n + 1, d.layers[n].name),
        Item::Object(n) => format!("Page object {}", n + 1),
        Item::Vector(id) => format!("Vector path {id}"),
        Item::Gradient(id) => format!("Gradient {id}"),
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::alignment_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for y in [4.0, 20.0] {
        let state = drawer_core::shapes::prepare(
            &app.documents[0],
            drawer_core::shapes::Target::Vector(None),
            &[[y, y], [y + 8.0, y + 8.0]],
            drawer_core::shapes::Settings {
                paint: drawer_core::shapes::Paint::CreateFill,
                ..Default::default()
            },
        )?;
        app.documents[0].commit_state(state)?;
    }
    let selected: Vec<_> = app.documents[0]
        .scene
        .lines
        .layers
        .iter()
        .flat_map(|l| &l.paths)
        .map(|p| Item::Vector(p.id))
        .collect();
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "align-objects");
    let e = app
        .alignment
        .draft
        .as_mut()
        .ok_or("Alignment command missing")?;
    e.selected = selected.clone();
    e.reference = Reference::Canvas;
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    let expected = core::prepare(
        &app.documents[0],
        &selected,
        Reference::Canvas,
        Operation::Align(Align::Left),
    )?;
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let rect = app.alignment.apply.ok_or("Alignment apply missing")?;
    for pressed in [true, false] {
        step(
            &mut app,
            &mut time,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    if app.documents[0].snapshot() != expected
        || app.documents[0].history_counts().0 != history.0 + 1
    {
        return Err(format!("Alignment production button: {:?}", app.status));
    }
    app.alignment.draft = None;
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Alignment production grouped Undo".into());
    }
    app.execute_command(&ctx, "align-objects");
    let before = app.documents[0].snapshot();
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
    if app.alignment.draft.is_some() || app.documents[0].snapshot() != before {
        return Err("Alignment cancel changed page".into());
    }
    app.execute_command(&ctx, "align-objects");
    app.documents[0].edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    let before = app.documents[0].snapshot();
    if app.apply_alignment(Operation::Align(Align::Left)).is_ok()
        || app.documents[0].snapshot() != before
    {
        return Err("Stale alignment applied".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn alignment_production_apply_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
