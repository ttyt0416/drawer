use super::*;
impl Drawer {
    pub(super) fn color_mode_command(&mut self, command: &str) -> Result<(), String> {
        self.drawing.cancel_sample();
        let current = self.references.session.workspace.color;
        let mode = &mut self.workspaces.store.current.color_modes;
        let color = match command {
            "primary-color" => Some(mode.select(0, current)?),
            "secondary-color" => Some(mode.select(1, current)?),
            "swap-colors" => Some(mode.swap(current)?),
            "transparent-color" => {
                mode.observe(current);
                mode.transparent = !mode.transparent;
                None
            }
            _ => return Err("Unknown drawing color command".into()),
        };
        if let Some(c) = color {
            self.choose_color(c)
        }
        self.drawing.transparent_mode = self.workspaces.store.current.color_modes.transparent;
        Ok(())
    }
    pub(super) fn color_mode_controls(&mut self, ui: &mut egui::Ui) {
        self.workspaces
            .store
            .current
            .color_modes
            .observe(self.references.session.workspace.color);
        let state = self.workspaces.store.current.color_modes;
        let mut command = None;
        ui.horizontal_wrapped(|ui| {
            for (id, label, selected) in [
                (
                    "primary-color",
                    "Primary",
                    state.active == 0 && !state.transparent,
                ),
                (
                    "secondary-color",
                    "Secondary",
                    state.active == 1 && !state.transparent,
                ),
                (
                    "transparent-color",
                    "Transparent shape eraser (C)",
                    state.transparent,
                ),
            ] {
                if ui.selectable_label(selected, label).clicked() {
                    command = Some(id)
                }
            }
            if ui.button("Swap colors (X)").clicked() {
                command = Some("swap-colors")
            }
        });
        ui.label(format!(
            "Primary {} · Secondary {}",
            drawer_core::color_selector::hex(state.slots[0]),
            drawer_core::color_selector::hex(state.slots[1])
        ));
        if let Some(c) = command {
            self.execute_command(ui.ctx(), c)
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
fn click(p: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(p),
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]
}
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::color_modes::verify(root)?;
    verify_mouse_colors(root)?;
    verify_pen()?;
    super::workspaces::verify_test_history(root)
}
fn verify_mouse_colors(root: &std::path::Path) -> Result<(), String> {
    let ctx = egui::Context::default();
    let user = root.join("color-modes-user");
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.0,
        background: Background::Color([80, 90, 100]),
    })
    .map_err(|e| e.to_string())?;
    let red = [220, 30, 40, 0];
    let blue = [20, 80, 160, 128];
    app.choose_color(red);
    app.color_mode_command("secondary-color")?;
    app.choose_color(blue);
    app.color_mode_command("primary-color")?;
    if app.references.session.workspace.color != red {
        return Err("Primary/secondary alpha lost".into());
    }
    app.panels.settings.brush_size = 8.0;
    app.panels.settings.brush_hardness[0] = 1.0;
    app.panels.settings.brush_opacity[0] = 1.0;
    app.workspaces.store.current.pressure.tools[0].size = false;
    app.workspaces.store.current.pressure.tools[0].opacity = false;
    app.workspaces.store.current.input.profiles[0].image = drawer_core::tip_profile::Image::Square;
    app.workspaces.store.current.input.smoothing[0] = 0.0;
    app.workspaces.store.current.input.clone_stamp.enabled = true;
    app.workspaces.store.current.input.materials[0].mode = drawer_core::brush_material::Mode::Water;
    app.workspaces.store.current.input.materials[0].alpha_lock = true;
    app.workspaces.store.current.input.materials[0].water = 1.0;
    app.workspaces.store.current.input.materials[0].edge = 1.0;
    let mut t = 0.0;
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![key(egui::Key::X, true, false)]);
    if app.references.session.workspace.color != blue {
        return Err("X color exchange failed".into());
    }
    step(&ctx, &mut app, &mut t, vec![key(egui::Key::X, true, true)]);
    if app.references.session.workspace.color != blue {
        return Err("Key repeat exchanged colors twice".into());
    }
    step(
        &ctx,
        &mut app,
        &mut t,
        vec![key(egui::Key::X, false, false)],
    );
    step(&ctx, &mut app, &mut t, vec![key(egui::Key::X, true, false)]);
    step(
        &ctx,
        &mut app,
        &mut t,
        vec![key(egui::Key::X, false, false)],
    );
    let before = app.documents[0].snapshot();
    let pos = super::precision::pointer(
        &ctx,
        &app,
        [16.5, 16.5],
        drawer_core::pointer_input::Phase::Down,
    )
    .position;
    let pos = egui::pos2(pos[0] as f32, pos[1] as f32);
    let mut down = click(pos, true);
    down.insert(0, key(egui::Key::C, true, false));
    step(&ctx, &mut app, &mut t, down);
    if !app.workspaces.store.current.color_modes.transparent || !app.drawing.pending() {
        return Err(format!(
            "Same-frame C and pointer down did not begin transparent stroke: {:?}",
            app.status
        ));
    }
    step(&ctx, &mut app, &mut t, vec![key(egui::Key::C, true, true)]);
    if !app.workspaces.store.current.color_modes.transparent
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("Transparent repeat toggled or ended the pending stroke".into());
    }
    let mut up = click(pos, false);
    up.push(key(egui::Key::C, false, false));
    step(&ctx, &mut app, &mut t, up);
    if app.drawing.pending()
        || !app.workspaces.input_idle()
        || app.documents[0].layers[0].pixel(32, 32, 19, 19) != [80, 90, 100, 0]
        || app.selected != BrushTip::Pen
    {
        return Err(format!(
            "Transparent square shape/release mismatch: {:?}; status {:?}",
            app.documents[0].layers[0].pixel(32, 32, 19, 19),
            app.status
        ));
    }
    let after = app.documents[0].snapshot();
    if app.documents[0].history_counts() != (1, 0)
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || !app.documents[0].redo()
        || app.documents[0].snapshot() != after
    {
        return Err("Production transparent stroke undo/redo mismatch".into());
    }
    let path = root.join("app-transparent-shape.drawer");
    drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != after
    {
        return Err("Production transparent native reopen mismatch".into());
    }
    app.workspaces.store.current.scratchpad.visible = true;
    app.workspaces.store.current.scratchpad.pixels[10] = [100, 80, 60, 128];
    app.persist_workspace()?;
    let wanted = app.capture_workspace();
    let restart = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    let restored = restart.capture_workspace();
    if restored.color_modes != wanted.color_modes
        || restored.color != wanted.color
        || restored.tool != wanted.tool
        || restored.scratchpad != wanted.scratchpad
        || !restored.color_modes.transparent
    {
        return Err("Colors/transparent mode/tool/test area restart mismatch".into());
    }
    Ok(())
}
fn verify_pen() -> Result<(), String> {
    use drawer_core::pointer_input::{Kind, Phase, Source};
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 32,
        height: 32,
        dpi: 300.0,
        background: Background::Color([80, 90, 100]),
    })
    .map_err(|e| e.to_string())?;
    app.panels.settings.brush_size = 8.0;
    app.panels.settings.brush_hardness[0] = 1.0;
    app.panels.settings.brush_opacity[0] = 1.0;
    app.workspaces.store.current.input.profiles[0].image = drawer_core::tip_profile::Image::Square;
    app.workspaces.store.current.pressure.tools[0].size = false;
    app.workspaces.store.current.pressure.tools[0].opacity = false;
    let mut t = 0.0;
    step(&ctx, &mut app, &mut t, vec![]);
    step(&ctx, &mut app, &mut t, vec![]);
    let mut down = super::precision::pointer(&ctx, &app, [16.5, 16.5], Phase::Down);
    down.source = Source {
        kind: Kind::Pen,
        device: 77,
        contact: 1,
    };
    down.pressure = Some(1.0);
    app.drawing.virtual_events(vec![down], true);
    step(&ctx, &mut app, &mut t, vec![key(egui::Key::C, true, false)]);
    let mut up = down;
    up.phase = Phase::Up;
    app.drawing.virtual_events(vec![up], true);
    step(
        &ctx,
        &mut app,
        &mut t,
        vec![key(egui::Key::C, false, false)],
    );
    if app.drawing.pending()
        || !app.workspaces.input_idle()
        || app.documents[0].layers[0].pixel(32, 32, 19, 19)[3] != 0
        || app.drawing.last_source.is_none_or(|s| s.kind != Kind::Pen)
    {
        return Err(format!(
            "Synthetic pen/C release: pending={}, idle={}, pixel={:?}, source={:?}, mode={}, status={:?}",
            app.drawing.pending(),
            app.workspaces.input_idle(),
            app.documents[0].layers[0].pixel(32, 32, 19, 19),
            app.drawing.last_source,
            app.workspaces.store.current.color_modes.transparent,
            app.status
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_colors_transparent_mouse_pen_release_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
