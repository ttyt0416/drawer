use super::*;
use drawer_core::alignment::{self as core, Align, Item, Operation, Reference};
#[derive(Default)]
pub(super) struct Panel {
    pub draft: Option<Box<Draft>>,
    apply: Option<egui::Rect>,
    distribute: Option<egui::Rect>,
}
pub(super) struct Draft {
    id: u64,
    revision: u64,
    frame: u32,
    selected: Vec<Item>,
    reference: Reference,
    options: drawer_core::alignment_detail::Options,
}
impl Drawer {
    pub(super) fn begin_alignment(&mut self) -> Result<(), String> {
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let saved = &d.scene.alignment;
        let items = core::items(d);
        let reference = match saved.reference {
            Reference::Key(i) if !items.contains(&i) => Reference::Selection,
            Reference::Guide(n) if n >= d.scene.guides.len() => Reference::Selection,
            r => r,
        };
        self.alignment.draft = Some(Box::new(Draft {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            selected: saved
                .selected
                .iter()
                .copied()
                .filter(|i| items.contains(i))
                .collect(),
            reference,
            options: saved.options,
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
        let state = core::prepare_advanced(
            d,
            &drawer_core::alignment_detail::Settings {
                selected: e.selected.clone(),
                reference: e.reference,
                options: e.options,
            },
            op,
        )?;
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
        let Some(n) = self.active.filter(|n| self.documents[*n].id == e.id) else {
            self.alignment.draft = None;
            return;
        };
        if self.documents[n].revision != e.revision || self.documents[n].current_frame != e.frame {
            let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            egui::Window::new("Align and distribute objects").show(ctx, |ui| {
                ui.label("Page/frame changed; cancel and reopen alignment.");
                cancel |= ui.button("Cancel").clicked();
            });
            if cancel {
                self.alignment.draft = None;
            }
            return;
        }
        let d = &self.documents[n];
        let items = core::items(d);
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut operation = None;
        self.alignment.apply = None;
        egui::Window::new("Align and distribute objects").open(&mut open).collapsible(false).default_width(560.0).min_width(560.0).default_pos(egui::pos2(60.0,20.0)).resizable(false).show(ctx,|ui|{
            egui::ScrollArea::vertical().id_salt("alignment-body").max_height(280.0).auto_shrink([false,false]).show(ui,|ui|{
            ui.horizontal(|ui|{ui.radio_value(&mut e.reference,Reference::Selection,"Selection bounds");ui.radio_value(&mut e.reference,Reference::Canvas,"Canvas");});
            egui::ComboBox::from_id_salt("alignment-key").selected_text(match e.reference{Reference::Key(i)=>format!("Key: {}",label(d,i)),_=>"Choose reference object".into()}).show_ui(ui,|ui|{for i in &items{ui.selectable_value(&mut e.reference,Reference::Key(*i),label(d,*i));}});
            egui::ComboBox::from_id_salt("alignment-guide").selected_text(match e.reference{Reference::Guide(n)=>format!("Page guide {}",n+1),_=>"Choose page guide".into()}).show_ui(ui,|ui|{for (n,_) in d.scene.guides.iter().enumerate(){ui.selectable_value(&mut e.reference,Reference::Guide(n),format!("Page guide {}",n+1));}});
            ui.checkbox(&mut e.options.text_pixels,"Align to text pixels (instead of text boxes)");ui.checkbox(&mut e.options.vector_paths,"Align to vector paths (instead of stroke outlines)");ui.checkbox(&mut e.options.raster_pixels,"Ignore transparent raster margins");
            ui.horizontal(|ui|{if ui.button("Select all").clicked(){e.selected=items.clone();}
if ui.button("Select none").clicked(){e.selected.clear();}});
            egui::ScrollArea::vertical().id_salt("alignment-items").max_height(120.0).show(ui,|ui|{for item in &items{let mut selected=e.selected.contains(item);if ui.checkbox(&mut selected,label(d,*item)).changed(){if selected{e.selected.push(*item)}else{e.selected.retain(|i|i!=item)}}}});
            });
            ui.horizontal_wrapped(|ui|{for a in Align::ALL{let r=ui.button(format!("{a:?}"));if a==Align::Left{self.alignment.apply=Some(r.rect);}
if r.clicked(){operation=Some(Operation::Align(a));}}});
            ui.horizontal_wrapped(|ui|{for (label,op) in [("Equal horizontal gaps",Operation::GapsHorizontal),("Equal vertical gaps",Operation::GapsVertical),("Equal horizontal centers",Operation::CentersHorizontal),("Equal vertical centers",Operation::CentersVertical)]{if ui.button(label).clicked(){operation=Some(op);}}});
            ui.horizontal_wrapped(|ui|{for a in Align::ALL{let r=ui.button(format!("Distribute {}",distribution_label(a)));if a==Align::Right{self.alignment.distribute=Some(r.rect);}
if r.clicked(){operation=Some(Operation::Edges(a));}}});
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
fn distribution_label(a: Align) -> &'static str {
    match a {
        Align::Left => "left edges",
        Align::Right => "right edges",
        Align::Top => "top edges",
        Align::Bottom => "bottom edges",
        Align::Center => "horizontal centers",
        Align::Middle => "vertical centers",
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
    let expected = core::prepare_advanced(
        &app.documents[0],
        &drawer_core::alignment_detail::Settings {
            selected: selected.clone(),
            reference: Reference::Canvas,
            options: Default::default(),
        },
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
pub(super) fn detail_smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::alignment_detail_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 64,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    for (x, w) in [(2.0, 4.0), (17.0, 10.0), (44.0, 16.0)] {
        app.documents[0]
            .scene
            .objects
            .push(drawer_core::scene::PageObject::Tone {
                bounds: drawer_core::scene::Rect {
                    min: [x, 8.0],
                    max: [x + w, 16.0],
                },
                period_px: 4.0,
                coverage: 0.5,
            });
    }
    let original = app.documents[0].snapshot();
    let mut time = 0.0;
    let step = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.1;
        super::input_replay::frame(&ctx, app, *time, events);
    };
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    app.execute_command(&ctx, "align-objects");
    app.alignment
        .draft
        .as_mut()
        .ok_or("Alignment draft missing")?
        .selected = vec![Item::Object(0), Item::Object(1), Item::Object(2)];
    step(&mut app, &mut time, vec![]);
    step(&mut app, &mut time, vec![]);
    let rect = app
        .alignment
        .distribute
        .ok_or("Right edge distribution button missing")?;
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
    if core::bounds(&app.documents[0], Item::Object(1))?.max[0] != 33.0
        || app.documents[0].history_counts().0 != 1
    {
        return Err(format!(
            "Right-edge production distribution: {:?} {rect:?}",
            app.status
        ));
    }
    let file = root.join("alignment-detail-ui.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .scene
        .alignment
        != app.documents[0].scene.alignment
    {
        return Err("Alignment UI settings reopen".into());
    }
    app.alignment.draft = None;
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Advanced distribution grouped Undo".into());
    }
    app.execute_command(&ctx, "align-objects");
    app.documents.clear();
    app.active = None;
    app.alignment_window(&ctx);
    if app.alignment.draft.is_some() {
        return Err("Closed alignment document retained draft".into());
    }
    for pen in [false, true] {
        for moving in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            app.create_canvas(CanvasSpec {
                width: 64,
                height: 64,
                dpi: 300.0,
                background: Background::Transparent,
            })
            .map_err(|e| e.to_string())?;
            app.documents[0].view.angle = 90.0;
            app.documents[0].view.zoom = 2.0;
            app.workspaces.store.current.snapping.canvas_edges = true;
            app.workspaces.store.current.snapping.screen_distance = 4.0;
            app.set_tool(BrushTip::PixelPen);
            app.panels.settings.brush_size = 1.0;
            app.choose_color([200, 40, 20, 255]);
            app.workspaces.store.current.pressure.tools[6].size = false;
            app.workspaces.store.current.pressure.tools[6].opacity = false;
            app.workspaces.store.current.input.profiles[6].minimum_size = 1.0;
            app.workspaces.store.current.input.profiles[6].minimum_opacity = 1.0;
            app.workspaces.store.current.input.profiles[6]
                .pixel
                .pressure = false;
            let points = if moving {
                app.drawing.mode = super::precision::Mode::Move;
                app.documents[0].layers[0].placement = Some(drawer_core::document::PlacedRaster {
                    width: 4,
                    height: 4,
                    offset: [3, 8],
                });
                app.documents[0].layers[0].pixels = vec![[200, 40, 20, 255]; 16];
                [[4.0, 9.0], [3.0, 9.0]]
            } else {
                [[1.5, 8.5], [1.5, 24.5]]
            };
            let before = app.documents[0].snapshot();
            let mut time = 0.0;
            let step = |app: &mut Drawer, time: &mut f64, events| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, events);
            };
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            for (phase, p) in [
                drawer_core::pointer_input::Phase::Down,
                drawer_core::pointer_input::Phase::Up,
            ]
            .into_iter()
            .zip(points)
            {
                let mut e = super::precision::pointer(&ctx, &app, p, phase);
                if pen {
                    e.source = drawer_core::pointer_input::Source {
                        kind: drawer_core::pointer_input::Kind::Pen,
                        device: 115,
                        contact: 1,
                    };
                    e.pressure = Some(0.5);
                }
                app.drawing.virtual_events(vec![e], pen);
                step(&mut app, &mut time, vec![]);
            }
            let d = &mut app.documents[0];
            let good = if moving {
                d.layers[0].extent(64, 64).offset == [0, 8]
                    && d.layers[0].pixels == before.layers[0].pixels
            } else {
                d.layers[0].pixel(64, 64, 0, 16)[3] > 0 && d.layers[0].pixel(64, 64, 1, 16)[3] == 0
            };
            if !good || d.history_counts().0 != 1 {
                return Err(format!(
                    "Canvas edge production pen={pen} move={moving}: {:?}",
                    app.status
                ));
            }
            if !d.undo() || d.snapshot() != before {
                return Err("Object snapping production Undo".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn detailed_alignment_and_canvas_edge_input() {
        let root = crate::tests::TestDirectory::new();
        super::detail_smoke(&root.0).unwrap();
    }
    #[test]
    fn alignment_production_apply_cancel_stale() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
