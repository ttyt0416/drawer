use super::*;
use drawer_core::{
    layer_move::{Settings, Source},
    pointer_input::Phase,
};
fn step(ctx: &egui::Context, app: &mut Drawer, time: &mut f64, events: Vec<egui::Event>) {
    *time += 0.1;
    super::input_replay::frame(ctx, app, *time, events);
}
fn event(
    ctx: &egui::Context,
    app: &mut Drawer,
    p: [f64; 2],
    phase: Phase,
) -> drawer_core::pointer_input::Event {
    super::precision::pointer(ctx, app, p, phase)
}
fn drag(
    ctx: &egui::Context,
    app: &mut Drawer,
    time: &mut f64,
    from: [f64; 2],
    to: [f64; 2],
) -> Result<(), String> {
    let original = app.documents[0].snapshot();
    for (p, phase) in [(from, Phase::Down), (to, Phase::Move)] {
        let e = event(ctx, app, p, phase);
        app.drawing.virtual_events(vec![e], false);
        step(ctx, app, time, vec![]);
    }
    if app.documents[0].snapshot() != original || !app.drawing.pending() {
        return Err(format!(
            "Movement pointer preview mutated source or missing: {:?}",
            app.status
        ));
    }
    let e = event(ctx, app, to, Phase::Up);
    app.drawing.virtual_events(vec![e], false);
    step(ctx, app, time, vec![]);
    step(ctx, app, time, vec![]);
    Ok(())
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::layer_move_verify::verify(root)?;
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
    app.documents[0].commit_state(drawer_core::layer_move_verify::fixture()?.snapshot())?;
    app.workspaces.store.current.layer_move = drawer_core::layer_move_verify::click_settings();
    app.workspaces.store.current.snapping.rulers = false;
    app.drawing.mode = super::precision::Mode::Move;
    app.precision.target = drawer_core::precision::Target::Layer;
    let mut time = 0.;
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    drag(&ctx, &mut app, &mut time, [2.5, 2.5], [4.5, 3.5])?;
    if app.documents[0].layers[0].extent(8, 8).offset != [2, 1]
        || app.documents[0].layers[1..] != original.layers[1..]
        || app.documents[0].scene.objects != original.scene.objects
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.panels.targets.get(&app.documents[0].id)
            != Some(&drawer_core::thumbnail::Target::Raster(0))
    {
        return Err(format!(
            "Clicked-layer UI exclusions/selection/Undo: {:?}",
            app.status
        ));
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Click movement Undo".into());
    }
    // Auto-select off must keep the original selection while the clicked lower source moves.
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(1),
    );
    app.workspaces.store.current.layer_move.auto_select = false;
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    drag(&ctx, &mut app, &mut time, [2.5, 2.5], [3.5, 2.5])?;
    if app.panels.targets.get(&app.documents[0].id)
        != Some(&drawer_core::thumbnail::Target::Raster(1))
        || app.documents[0].layers[0].extent(8, 8).offset != [1, 0]
    {
        return Err("Auto-select off canceled movement or changed selected layer".into());
    }
    app.documents[0].undo();
    // Area mode chooses intersecting whole layers; Keep original moves new copies only.
    let mut scene = app.documents[0].scene.clone();
    scene.selection = Some(drawer_core::scene::Rect {
        min: [4., 3.],
        max: [6., 5.],
    });
    app.documents[0].edit_scene(scene)?;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    app.workspaces.store.current.layer_move = Settings {
        source: Source::Area,
        keep_original: true,
        ..drawer_core::layer_move_verify::click_settings()
    };
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    drag(&ctx, &mut app, &mut time, [1.5, 1.5], [3.5, 1.5])?;
    if app.documents[0].layers.len() != 5
        || app.documents[0].layers[..3] != original.layers[..]
        || app.documents[0].history_counts().0 != counts.0 + 1
        || app.panels.targets.get(&app.documents[0].id)
            != Some(&drawer_core::thumbnail::Target::Raster(3))
    {
        return Err(format!(
            "Area/copy UI original preservation or copy selection: {:?}",
            app.status
        ));
    }
    let path = root.join("pointer-move-copies.drawer");
    let expected = app.documents[0].snapshot();
    drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != expected
    {
        return Err("Pointer move copies native roundtrip".into());
    }
    app.documents[0].undo();
    // Every cancellation discards prospective copies and releases input.
    for cancel in 0..5 {
        app.panels.targets.insert(
            app.documents[0].id,
            drawer_core::thumbnail::Target::Raster(0),
        );
        step(&ctx, &mut app, &mut time, vec![]);
        step(&ctx, &mut app, &mut time, vec![]);
        let original = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let view = app.documents[0].view;
        let frame = app.documents[0].current_frame;
        let settings = app.workspaces.store.current.layer_move;
        for (p, phase) in [([1.5, 1.5], Phase::Down), ([3.5, 1.5], Phase::Move)] {
            let e = event(&ctx, &mut app, p, phase);
            app.drawing.virtual_events(vec![e], false);
            step(&ctx, &mut app, &mut time, vec![]);
        }
        let events = match cancel {
            0 => vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            1 => vec![egui::Event::WindowFocused(false)],
            2 => {
                app.workspaces.store.current.layer_move.auto_select = !settings.auto_select;
                vec![]
            }
            3 => {
                app.documents[0].current_frame += 1;
                vec![]
            }
            _ => {
                app.documents[0].view.zoom *= 1.5;
                vec![]
            }
        };
        step(&ctx, &mut app, &mut time, events);
        if app.documents[0].snapshot() != original
            || app.documents[0].history_counts() != counts
            || app.drawing.pending()
        {
            return Err(format!(
                "Copy gesture cancellation {cancel} changed source or retained input: {:?}",
                app.status
            ));
        }
        let up = event(&ctx, &mut app, [3.5, 1.5], Phase::Up);
        app.drawing.virtual_events(vec![up], false);
        step(
            &ctx,
            &mut app,
            &mut time,
            vec![egui::Event::WindowFocused(true)],
        );
        app.documents[0].view = view;
        app.documents[0].current_frame = frame;
        app.workspaces.store.current.layer_move = settings;
    }
    // Selection pixel copies use only the current raster, regardless of area targeting.
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 3,
        height: 1,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.drawing.mode = super::precision::Mode::Move;
    app.workspaces.store.current.layer_move = Settings {
        source: Source::Area,
        keep_original: true,
        ..Default::default()
    };
    app.workspaces.store.current.snapping.rulers = false;
    app.documents[0].commit_state(drawer_core::layer_masks_verify::fixture()?.snapshot())?;
    app.panels.targets.insert(
        app.documents[0].id,
        drawer_core::thumbnail::Target::Raster(0),
    );
    app.precision.target = drawer_core::precision::Target::Pixels;
    step(&ctx, &mut app, &mut time, vec![]);
    step(&ctx, &mut app, &mut time, vec![]);
    let original = app.documents[0].snapshot();
    drag(&ctx, &mut app, &mut time, [1.5, 0.5], [3.5, 1.5])?;
    if app.documents[0].layers.len() != 2
        || app.documents[0].layers[0] != original.layers[0]
        || app.documents[0].layers[1].pixel(3, 1, 3, 1)[3] != 64
        || app.documents[0].layers[1].pixel(3, 1, 4, 1)[3] != 128
        || app.panels.targets.get(&app.documents[0].id)
            != Some(&drawer_core::thumbnail::Target::Raster(1))
    {
        return Err(format!(
            "Selected pixel copy UI strength/source/selection: {:?}",
            app.status
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mouse_clicked_area_copy_auto_select_cancel_and_selected_pixels() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
