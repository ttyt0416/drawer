use super::*;
use drawer_core::selection_tools as core;
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    name: String,
}
impl Drawer {
    pub(super) fn selection_command(&mut self, command: &str) -> Result<(), String> {
        use super::precision::Mode;
        match command {
            "select-rectangle" | "select-lasso" => {
                self.restore_locked_tool();
                self.drawing.mode = if command == "select-rectangle" {
                    Mode::SelectRectangle
                } else {
                    Mode::SelectLasso
                };
                self.drawing.connect_line = false;
                return Ok(());
            }
            "selection-tools" => {
                self.selection_panel.open = true;
                return Ok(());
            }
            _ => {}
        }
        let n = self.active.ok_or("Open a page first")?;
        if command == "duplicate-page" {
            let page = self.documents[n].fork_page()?;
            self.add_document(page);
            return Ok(());
        }
        let d = &mut self.documents[n];
        match command {
            "selection-clear" => {
                core::clear(d)?;
            }
            "selection-invert" => {
                core::invert(d)?;
            }
            "selection-reselect" => {
                core::reselect(d)?;
            }
            "selection-save" => {
                core::save_named(d, &self.selection_panel.name)?;
            }
            _ => return Err("Unknown selection command".into()),
        }
        Ok(())
    }
    pub(super) fn selection_window(&mut self, ctx: &egui::Context) {
        if !self.selection_panel.open {
            return;
        }
        let mut open = true;
        let mut command = None;
        let mut named = None;
        egui::Window::new("Page selections").open(&mut open).vscroll(true).show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{for (id,label) in [("select-rectangle","Rectangle"),("select-lasso","Lasso"),("selection-clear","Deselect"),("selection-invert","Invert"),("selection-reselect","Reselect")]{if ui.button(label).clicked(){command=Some(id);}}});
            ui.label("Drag on the canvas and release to select. Escape/focus loss cancels the draft. Edges use 4×4 coverage samples; no painting pixels change.");
            let Some(n)=self.active else{ui.label("Open a page first");return;};let d=&self.documents[n];
            ui.label(format!("Page: {} · frame {}. Selection stays with this page when cells/frames change.",d.title,d.current_frame+1));
            super::edit::text(ui,&mut self.selection_panel.name,"selection-tools");if ui.button("Save / replace named page selection").clicked(){command=Some("selection-save");}
            for s in &d.scene.selections.saved{ui.push_id((d.id,s.id),|ui|{ui.horizontal_wrapped(|ui|{ui.label(&s.name);if ui.button("Load selection").clicked(){named=Some((s.id,true));}
            if ui.button("Remove saved selection").clicked(){named=Some((s.id,false));}});});}
            if ui.button("Duplicate page as separate document").clicked(){command=Some("duplicate-page");}
            ui.label("Selection changes use drawing Undo/Redo. Named selections and the previous selection are saved in the page. A copied page receives independent selection identity.");
        });
        self.selection_panel.open = open;
        if let Some(c) = command {
            self.execute_command(ctx, c);
        }
        if let Some((id, load)) = named {
            self.finish_stroke();
            if let Some(n) = self.active {
                let d = &mut self.documents[n];
                let result = if load {
                    core::load_named(d, id)
                } else {
                    core::delete_named(d, id)
                };
                if let Err(e) = result {
                    self.status = Some(e);
                }
            }
        }
    }
}

fn key(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Default::default(),
    }
}
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
fn pointer(
    ctx: &egui::Context,
    app: &Drawer,
    p: [f64; 2],
    phase: drawer_core::pointer_input::Phase,
    pen: bool,
) -> drawer_core::pointer_input::Event {
    let mut e = super::precision::pointer(ctx, app, p, phase);
    if pen {
        e.source = drawer_core::pointer_input::Source {
            kind: drawer_core::pointer_input::Kind::Pen,
            device: 71,
            contact: 9,
        };
        e.pressure = Some(0.5);
    }
    e
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::pointer_input::Phase;
    core::verify_pages(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    let mut time = 0.0;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 24,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    drawer_core::files::save(
        &mut app.documents[0],
        &root.join("select-input-base.drawer"),
    )
    .map_err(|e| e.to_string())?;
    let before = app.documents[0].snapshot();
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    app.execute_command(&ctx, "select-rectangle");
    for (p, phase) in [([2.25, 3.5], Phase::Down), ([18.75, 20.0], Phase::Move)] {
        let e = pointer(&ctx, &app, p, phase, false);
        app.drawing.virtual_events(vec![e], false);
        step(&ctx, &mut app, &mut time, vec![]);
        if app.documents[0].snapshot() != before || !app.drawing.pending() {
            return Err("Selection preview edited data or did not capture input".into());
        }
    }
    if app.drawing.selection_preview(app.documents[0].id).is_none() {
        return Err("Selection outline preview missing".into());
    }
    let e = pointer(&ctx, &app, [18.75, 20.0], Phase::Up, false);
    app.drawing.virtual_events(vec![e], false);
    step(&ctx, &mut app, &mut time, vec![]);
    if app.drawing.pending()
        || app.documents[0].history_counts() != (1, 0)
        || app.documents[0].scene.selection_coverage([2.5, 3.5]) != 96
    {
        return Err(format!(
            "Mouse selection did not commit on release: {:?}",
            app.status
        ));
    }
    let selected = app.documents[0].snapshot();
    if !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != selected
    {
        return Err("Selection production Undo/Redo failed".into());
    }
    app.selection_panel.name = "First page area".into();
    app.execute_command(&ctx, "selection-save");
    let first = app.documents[0].scene.selection_mask.clone();
    app.execute_command(&ctx, "selection-clear");
    app.execute_command(&ctx, "selection-reselect");
    if app.documents[0].scene.selection_mask != first {
        return Err("Selection clear/reselect lost page mask".into());
    }
    app.execute_command(&ctx, "select-lasso");
    for (p, phase) in [
        ([4.0, 4.0], Phase::Down),
        ([24.0, 4.0], Phase::Move),
        ([24.0, 16.0], Phase::Move),
        ([4.0, 16.0], Phase::Move),
        ([4.0, 4.0], Phase::Up),
    ] {
        let e = pointer(&ctx, &app, p, phase, true);
        app.drawing.virtual_events(vec![e], true);
        step(&ctx, &mut app, &mut time, vec![]);
    }
    if app.drawing.pending()
        || app.documents[0].scene.selection_coverage([8.5, 8.5]) != 255
        || app.documents[0].scene.selection_coverage([25.5, 8.5]) != 0
    {
        return Err(format!(
            "Synthetic pen lasso release failed: {:?}",
            app.status
        ));
    }
    let selected = app.documents[0].snapshot();
    for focus in [false, true] {
        app.execute_command(&ctx, "select-rectangle");
        let e = pointer(&ctx, &app, [3.0, 3.0], Phase::Down, false);
        app.drawing.virtual_events(vec![e], false);
        step(&ctx, &mut app, &mut time, vec![]);
        let events = if focus {
            vec![egui::Event::WindowFocused(false)]
        } else {
            vec![key(egui::Key::Escape, true, false)]
        };
        step(&ctx, &mut app, &mut time, events);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![
                key(egui::Key::Escape, false, false),
                egui::Event::WindowFocused(true),
            ],
        );
        if app.drawing.pending() || app.documents[0].snapshot() != selected {
            return Err("Escape/focus loss committed selection draft".into());
        }
    }
    app.set_tool(BrushTip::Pen);
    app.workspaces
        .store
        .current
        .bindings
        .retain(|b| !(b.key == "R" && !b.ctrl && !b.shift && !b.alt));
    app.workspaces
        .store
        .current
        .bindings
        .push(drawer_core::workspaces::Binding {
            key: "R".into(),
            ctrl: false,
            shift: false,
            alt: false,
            command: "select-rectangle".into(),
        });
    let e = pointer(&ctx, &app, [1.0, 1.0], Phase::Down, true);
    app.drawing.virtual_events(vec![e], true);
    step(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::R, true, false)],
    );
    let e = pointer(&ctx, &app, [12.0, 12.0], Phase::Move, true);
    app.drawing.virtual_events(vec![e], true);
    step(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::R, true, true)],
    );
    if !app.drawing.pending() {
        return Err("Selection shortcut repeat cancelled pen contact".into());
    }
    let e = pointer(&ctx, &app, [12.0, 12.0], Phase::Up, true);
    app.drawing.virtual_events(vec![e], true);
    step(
        &ctx,
        &mut app,
        &mut time,
        vec![key(egui::Key::R, false, false)],
    );
    if app.drawing.pending()
        || !app.workspaces.input_idle()
        || app.documents[0].scene.selection_coverage([2.5, 2.5]) != 255
    {
        return Err("Simultaneous pen/selection key release remained stuck".into());
    }
    let selected = app.documents[0].snapshot();
    let e = pointer(&ctx, &app, [2.0, 2.0], Phase::Down, false);
    app.drawing.virtual_events(vec![e], false);
    step(&ctx, &mut app, &mut time, vec![]);
    app.duplicate_view();
    step(&ctx, &mut app, &mut time, vec![]);
    if app.drawing.pending() || app.documents[0].snapshot() != selected {
        return Err("View switch committed pending selection".into());
    }
    if app.documents[0].layers != before.layers {
        return Err("Selection tools painted raster pixels".into());
    }
    app.documents[0].view.zoom = 1.5;
    app.documents[0].view.angle = 90.0;
    step(&ctx, &mut app, &mut time, vec![]);
    app.execute_command(&ctx, "select-rectangle");
    let e = pointer(&ctx, &app, [4.0, 4.0], Phase::Down, false);
    app.drawing.virtual_events(vec![e], false);
    step(&ctx, &mut app, &mut time, vec![]);
    let e = pointer(&ctx, &app, [-500.0, 500.0], Phase::Up, false);
    app.drawing.virtual_events(vec![e], false);
    step(&ctx, &mut app, &mut time, vec![]);
    if app.drawing.pending()
        || app.documents[0].scene.selection_coverage([1.5, 8.5]) != 255
        || app.documents[0].scene.selection_coverage([8.5, 8.5]) != 0
    {
        return Err("Rotated/zoomed selection release outside viewport failed".into());
    }
    let selected = app.documents[0].snapshot();
    let e = pointer(&ctx, &app, [2.0, 2.0], Phase::Down, false);
    app.drawing.virtual_events(vec![e], false);
    step(&ctx, &mut app, &mut time, vec![]);
    app.pressure.open = true;
    let e = pointer(&ctx, &app, [12.0, 12.0], Phase::Up, false);
    app.drawing.virtual_events(vec![e], false);
    step(&ctx, &mut app, &mut time, vec![]);
    app.pressure.open = false;
    if app.drawing.pending() || app.documents[0].snapshot() != selected {
        return Err("Modal input committed selection draft".into());
    }
    app.execute_command(&ctx, "duplicate-page");
    if app.documents.len() != 2
        || app.documents[0].scene.selections.page == app.documents[1].scene.selections.page
    {
        return Err("Page duplicate reused selection identity".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_rectangle_pen_lasso_cancel_same_frame_keys_and_page_copy() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
