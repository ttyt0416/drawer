use super::*;
use drawer_core::{
    brush_behavior::Settings,
    pointer_input::{Kind, Phase, Source},
};
fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64, ppp: f32, events: Vec<egui::Event>) {
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        time: Some(time),
        events,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(ppp);
    let mut out = ctx.run_ui(input, |ui| app.show(ui));
    out.textures_delta.clear();
}
fn setup(zoom: f64, ppp: f32) -> Result<(egui::Context, Drawer), String> {
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.create_canvas(CanvasSpec {
        width: 64,
        height: 48,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.simple_view = true;
    app.documents[0].view.zoom = zoom;
    app.references.session.workspace.color = [220, 70, 30, 64];
    app.workspaces.store.current.pressure.tools[0].size = false;
    app.workspaces.store.current.pressure.tools[0].opacity = true;
    frame(&ctx, &mut app, 0.0, ppp, vec![]);
    frame(&ctx, &mut app, 0.01, ppp, vec![]);
    Ok((ctx, app))
}
fn event(
    ctx: &egui::Context,
    app: &Drawer,
    phase: Phase,
    kind: Kind,
    micros: u64,
) -> drawer_core::pointer_input::Event {
    let mut e = super::precision::pointer(ctx, app, [32.5, 24.5], phase);
    e.source = if kind == Kind::Mouse {
        Source::MOUSE
    } else {
        Source {
            kind,
            device: 71,
            contact: 1,
        }
    };
    e.pressure = (kind == Kind::Pen).then_some(0.5);
    e.attributes.time_micros = Some(micros);
    e
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::brush_behavior::verify(root)?;
    verify_sizing()?;
    verify_minimum()?;
    verify_scatter()?;
    verify_timing()?;
    verify_settings(root)
}
fn verify_sizing() -> Result<(), String> {
    for ppp in [1.0, 2.0] {
        for zoom in [0.5, 2.0] {
            for screen in [false, true] {
                let (ctx, mut app) = setup(zoom, ppp)?;
                app.set_quick_brush_size(16.0)?;
                app.workspaces.store.current.input.patterns[0]
                    .behavior
                    .screen_size = screen;
                let e = event(&ctx, &app, Phase::Down, Kind::Mouse, 0);
                app.drawing.virtual_events(vec![e], false);
                frame(&ctx, &mut app, 1.0, ppp, vec![]);
                let expected = if screen { 16.0 / zoom } else { 16.0 };
                let style = app
                    .drawing
                    .preview(app.documents[0].id)
                    .ok_or("Sizing stroke did not start")?
                    .style();
                if style.shape.size != expected {
                    return Err("Actual drawing did not resolve physical screen size".into());
                }
                app.workspaces.store.current.input.patterns[0]
                    .behavior
                    .screen_size = !screen;
                app.set_quick_brush_size(3.0)?;
                if app
                    .drawing
                    .preview(app.documents[0].id)
                    .is_none_or(|s| s.style() != style)
                {
                    return Err("Size settings changed an existing pending stroke".into());
                }
                let e = event(&ctx, &app, Phase::Up, Kind::Mouse, 1000);
                app.drawing.virtual_events(vec![e], false);
                frame(&ctx, &mut app, 1.001, ppp, vec![]);
                if app.documents[0].history_counts() != (1, 0)
                    || app.documents[0].layers[0].pixels.iter().all(|p| p[3] == 0)
                {
                    return Err("Resolved size did not paint one stroke".into());
                }
            }
        }
    }
    Ok(())
}
fn verify_minimum() -> Result<(), String> {
    for enabled in [false, true] {
        let (ctx, mut app) = setup(1.0, 1.0)?;
        app.set_quick_brush_size(0.1)?;
        app.workspaces.store.current.input.patterns[0]
            .behavior
            .minimum_pixel = enabled;
        let e = event(&ctx, &app, Phase::Down, Kind::Mouse, 0);
        app.drawing.virtual_events(vec![e], false);
        frame(&ctx, &mut app, 1.0, 1.0, vec![]);
        if app
            .drawing
            .preview(app.documents[0].id)
            .ok_or("Minimum-size stroke unavailable")?
            .shape()
            .size
            != if enabled { 1.0 } else { 0.1 }
        {
            return Err("Actual minimum size option did not change geometry".into());
        }
    }
    Ok(())
}
fn verify_scatter() -> Result<(), String> {
    {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let mut seq = drawer_core::input_replay::Sequence::default();
        seq.patterns[0].mode = drawer_core::brush_pattern::Mode::Scatter;
        seq.patterns[0].behavior = Settings {
            continuous: true,
            fixed_scatter: true,
            ..Default::default()
        };
        seq.patterns[0].seed = 94;
        for (n, sample) in seq.samples.iter_mut().enumerate() {
            sample.micros = n as u64 * 50000;
            sample.event.position = [32.5, 24.5];
            sample.event.pressure = Some(1.0);
        }
        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        let a = super::input_replay::run(&seq, &cancel, &progress)?;
        let b = super::input_replay::run(&seq, &cancel, &progress)?;
        seq.patterns[0].seed += 1;
        let c = super::input_replay::run(&seq, &cancel, &progress)?;
        if !a.record.passed
            || !b.record.passed
            || !c.record.passed
            || a.pixels != b.pixels
            || a.pixels == c.pixels
        {
            return Err("Fixed scatter production seed replay failed".into());
        }
    }
    Ok(())
}
fn verify_timing() -> Result<(), String> {
    for kind in [Kind::Mouse, Kind::Pen] {
        let (ctx, mut app) = setup(1.0, 1.0)?;
        app.set_quick_brush_size(1.0)?;
        app.workspaces.store.current.input.patterns[0].behavior = Settings {
            continuous: true,
            rate: 20,
            ..Default::default()
        };
        let e = event(&ctx, &app, Phase::Down, kind, 0);
        app.drawing.virtual_events(vec![e], kind == Kind::Pen);
        frame(&ctx, &mut app, 1.0, 1.0, vec![]);
        app.workspaces.store.current.input.patterns[0].behavior.rate = 200;
        app.drawing.virtual_events(vec![], kind == Kind::Pen);
        frame(&ctx, &mut app, 1.1, 1.0, vec![]);
        let d = &app.documents[0];
        let preview = app
            .drawing
            .preview(d.id)
            .ok_or("Held brush lost its session")?;
        super::workspaces::verify_continuous_pad(preview.style())?;
        let alpha = preview.pixel(d, 0, &d.layers[0], 32, 24).unwrap_or([0; 4])[3];
        let expected = if kind == Kind::Pen { 84 } else { 148 };
        if alpha != expected {
            return Err(format!(
                "Stationary {kind:?} emission/frozen rate: alpha {alpha}, expected {expected}"
            ));
        }
        let e = event(&ctx, &app, Phase::Up, kind, 100000);
        app.drawing.virtual_events(vec![e], kind == Kind::Pen);
        frame(&ctx, &mut app, 1.11, 1.0, vec![]);
        let committed = app.documents[0].snapshot();
        app.drawing.virtual_events(vec![], kind == Kind::Pen);
        frame(&ctx, &mut app, 2.0, 1.0, vec![]);
        if app.drawing.pending()
            || app.documents[0].snapshot() != committed
            || app.documents[0].history_counts() != (1, 0)
        {
            return Err("Release retained timed emission or split history".into());
        }
        let e = event(&ctx, &app, Phase::Down, kind, 200000);
        app.drawing.virtual_events(vec![e], kind == Kind::Pen);
        frame(&ctx, &mut app, 3.0, 1.0, vec![]);
        app.drawing.virtual_events(vec![], kind == Kind::Pen);
        frame(&ctx, &mut app, 3.1, 1.0, vec![]);
        app.drawing.virtual_events(vec![], kind == Kind::Pen);
        frame(
            &ctx,
            &mut app,
            3.11,
            1.0,
            vec![egui::Event::WindowFocused(false)],
        );
        if app.drawing.pending() || app.documents[0].snapshot() != committed {
            return Err("Focus loss left emission alive or published cancelled pixels".into());
        }
    }
    Ok(())
}
fn verify_settings(root: &std::path::Path) -> Result<(), String> {
    let user = root.join("behavior-user");
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.sync_tool_properties();
    let mut s = app.capture_workspace();
    let behavior = Settings {
        screen_size: true,
        minimum_pixel: true,
        spacing_density: true,
        continuous: true,
        rate: 57,
        fixed_scatter: true,
    };
    s.input.patterns[0].behavior = behavior;
    let preset =
        drawer_core::brush_presets::Preset::capture(&s, BrushTip::Pen, "Timed fixed brush")?;
    if drawer_core::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        != preset
    {
        return Err("Behavior preset round trip lost fields".into());
    }
    app.stage_workspace(s, app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted.workspaces.store.current.input.patterns[0].behavior != behavior {
        return Err("Brush behavior settings lost on restart".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn actual_sizing_stationary_mouse_pen_focus_loss_and_settings() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
