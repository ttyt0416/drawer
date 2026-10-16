use super::*;
use drawer_core::{
    material_library::Library,
    tip_texture::{Mode, Settings, Space},
};
pub(super) fn option_ui(ui: &mut egui::Ui, value: &mut Option<Settings>, library: &Library) {
    let mut enabled = value.is_some();
    if ui
        .checkbox(
            &mut enabled,
            "Override inherited brush texture for this tip",
        )
        .changed()
    {
        *value = enabled.then_some(Settings::default());
    }
    let Some(s) = value else {
        return;
    };
    ui.push_id("tip-texture-controls",|ui|{
        super::texture_panel::brush_pattern_ui(ui,&mut s.pattern,library);
        egui::ComboBox::from_id_salt("texture-mode").selected_text(s.mode.label()).show_ui(ui,|ui|{for mode in Mode::ALL{ui.selectable_value(&mut s.mode,mode,mode.label());}});
        ui.horizontal(|ui|{ui.selectable_value(&mut s.space,Space::Document,"Document coordinates");ui.selectable_value(&mut s.space,Space::Tip,"Tip coordinates (128px grid)");});
        ui.horizontal(|ui|{ui.checkbox(&mut s.flip[0],"Flip texture left/right");ui.checkbox(&mut s.flip[1],"Flip texture up/down");});
        super::edit::slider(ui,&mut s.brightness,-1.0..=1.0,"Texture brightness","tip-texture");
        super::edit::slider(ui,&mut s.contrast,-1.0..=1.0,"Texture contrast","tip-texture");
        ui.label("These modes change tip density. Outline uses local texture edges; Height averages density and texture height. A texture override with No texture disables the inherited texture.");
    });
}

fn frame(ctx: &egui::Context, app: &mut Drawer, time: f64, events: Vec<egui::Event>) {
    super::input_replay::frame(ctx, app, time, events);
}
fn setup() -> Result<(egui::Context, Drawer), String> {
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
    app.references.session.workspace.color = [180, 100, 50, 255];
    app.workspaces.store.current.pressure.tools[0].size = false;
    app.workspaces.store.current.pressure.tools[0].opacity = false;
    app.workspaces.store.current.input.smoothing[0] = 0.0;
    frame(&ctx, &mut app, 0.0, vec![]);
    frame(&ctx, &mut app, 0.1, vec![]);
    Ok((ctx, app))
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    let library = drawer_core::tip_texture::verify(root)?;
    verify_textures(&library)?;
    verify_views(root, &library)?;
    verify_termination()?;
    verify_settings(root, &library)
}
fn verify_textures(library: &Library) -> Result<(), String> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    };
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut images = vec![];
    for mode in Mode::ALL {
        let mut seq = drawer_core::input_replay::Sequence::default();
        seq.pressure.tools[0].size = false;
        seq.pressure.tools[0].opacity = false;
        seq.sequences[0].enabled = true;
        seq.sequences[0].count = 2;
        seq.sequences[0].tips[0] = library.assets()[0].id;
        seq.sequences[0].tips[1] = library.assets()[0].id;
        seq.sequences[0].slots[0].texture = Some(Settings {
            mode,
            pattern: drawer_core::texture::Pattern {
                source: library.assets()[1].id,
                strength: 0.8,
                scale: 3.0,
                ..Default::default()
            },
            ..Default::default()
        });
        seq.sequences[0].slots[1].texture = Some(Settings {
            mode,
            space: Space::Tip,
            pattern: drawer_core::texture::Pattern {
                source: library.assets()[1].id,
                strength: 0.8,
                scale: 1.5,
                angle: 90.0,
                ..Default::default()
            },
            ..Default::default()
        });
        for sample in &mut seq.samples {
            sample.event.pressure = Some(1.0);
        }
        let outcome = super::input_replay::run_with_materials(
            &seq,
            Arc::new(library.clone()),
            &cancel,
            &progress,
        )?;
        if !outcome.record.passed {
            return Err(format!("Tip texture production mode {mode:?}"));
        }
        if !images.contains(&outcome.pixels) {
            images.push(outcome.pixels);
        }
    }
    if images.len() < 6 {
        return Err("Texture density modes did not differ in production drawing".into());
    }
    Ok(())
}
fn verify_views(root: &std::path::Path, library: &Library) -> Result<(), String> {
    use drawer_core::pointer_input::Phase;
    let mut baseline = None;
    for (n, (zoom, angle, flipped)) in [(1.0, 0.0, false), (2.0, 37.0, true), (0.5, -31.0, false)]
        .into_iter()
        .enumerate()
    {
        let (ctx, mut app) = setup()?;
        app.materials.library = std::sync::Arc::new(library.clone());
        app.set_quick_brush_size(8.0)?;
        app.documents[0].view.zoom = zoom;
        app.documents[0].view.angle = angle;
        app.documents[0].view.flipped = flipped;
        app.workspaces.store.current.input.sequences[0].enabled = true;
        app.workspaces.store.current.input.sequences[0].tips[0] = library.assets()[0].id;
        app.workspaces.store.current.input.sequences[0].slots[0].texture = Some(Settings {
            space: Space::Tip,
            pattern: drawer_core::texture::Pattern {
                source: library.assets()[1].id,
                scale: 3.0,
                angle: 45.0,
                strength: 1.0,
                ..Default::default()
            },
            ..Default::default()
        });
        frame(&ctx, &mut app, 0.2, vec![]);
        for (step, phase) in [Phase::Down, Phase::Up].into_iter().enumerate() {
            let mut event = super::precision::pointer(&ctx, &app, [32.5, 24.5], phase);
            event.attributes.time_micros = Some(step as u64 * 1000);
            app.drawing.virtual_events(vec![event], false);
            frame(&ctx, &mut app, 1.0 + step as f64 * 0.001, vec![]);
        }
        let doc = &app.documents[0];
        if doc.history_counts() != (1, 0) {
            return Err("Texture view test lost its one stroke".into());
        }
        let pixels = doc.layers[0].pixels.clone();
        if let Some(before) = &baseline {
            if *before != pixels {
                return Err(
                    "Display zoom/rotation/mirror changed tip texture document pixels".into(),
                );
            }
        } else {
            baseline = Some(pixels);
        }
        if n == 0 {
            if !app.material_dependencies(doc).is_empty() {
                return Err("Registered per-tip texture reported missing".into());
            }
            drawer_core::files::save(&mut app.documents[0], &root.join("tip-texture-view.drawer"))
                .map_err(|e| e.to_string())?;
            app.materials.library = Default::default();
            if app.material_dependencies(&app.documents[0]).is_empty() {
                return Err("Missing per-tip dependency was hidden".into());
            }
        }
    }
    Ok(())
}
fn verify_termination() -> Result<(), String> {
    use drawer_core::pointer_input::Phase;
    let mut expected = None;
    for end in 0..3 {
        let (ctx, mut app) = setup()?;
        app.set_quick_brush_size(4.0)?;
        app.workspaces.store.current.input.materials[0] = drawer_core::brush_material::Settings {
            mode: drawer_core::brush_material::Mode::Water,
            edge: 1.0,
            water: 0.0,
            mixing: 0.0,
            retention: 0.0,
            supply: 1.0,
            ..Default::default()
        };
        let original = app.documents[0].snapshot();
        let mut event = super::precision::pointer(&ctx, &app, [32.5, 24.5], Phase::Down);
        event.attributes.time_micros = Some(0);
        app.drawing.virtual_events(vec![event], false);
        frame(&ctx, &mut app, 1.0, vec![]);
        if !app.drawing.pending() || app.documents[0].snapshot() != original {
            return Err("Water preview published early".into());
        }
        match end {
            0 => {
                let mut event = super::precision::pointer(&ctx, &app, [32.5, 24.5], Phase::Up);
                event.attributes.time_micros = Some(1000);
                app.drawing.virtual_events(vec![event], false);
                frame(&ctx, &mut app, 1.01, vec![]);
            }
            1 => {
                app.drawing.virtual_events(vec![], false);
                frame(
                    &ctx,
                    &mut app,
                    1.01,
                    vec![egui::Event::WindowFocused(false)],
                );
            }
            _ => {
                app.drawing.virtual_events(vec![], false);
                frame(
                    &ctx,
                    &mut app,
                    1.01,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
            }
        }
        if app.drawing.pending() {
            return Err("Water termination retained a session".into());
        }
        if end == 2 {
            if app.documents[0].snapshot() != original
                || app.documents[0].history_counts() != (0, 0)
            {
                return Err("Escape retained watercolor edge pixels".into());
            }
        } else {
            let doc = &mut app.documents[0];
            if doc.history_counts() != (1, 0) {
                return Err(format!(
                    "Water end {end} history {:?}; status {:?}",
                    doc.history_counts(),
                    app.status
                ));
            }
            let after = doc.snapshot();
            if let Some(before) = &expected {
                if *before != doc.layers[0].pixels {
                    return Err("Focus-loss edge differs from normal release".into());
                }
            } else {
                expected = Some(doc.layers[0].pixels.clone());
            }
            if !doc.undo() || doc.snapshot() != original || !doc.redo() || doc.snapshot() != after {
                return Err("Water termination postprocessing did not undo atomically".into());
            }
        }
    }
    Ok(())
}
fn verify_settings(root: &std::path::Path, library: &Library) -> Result<(), String> {
    let user = root.join("tip-texture-user");
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.publish_materials(library.clone())?;
    app.sync_tool_properties();
    let mut s = app.capture_workspace();
    let texture = Settings {
        space: Space::Tip,
        flip: [true, false],
        brightness: 0.2,
        contrast: -0.3,
        mode: Mode::Outline,
        pattern: drawer_core::texture::Pattern {
            source: library.assets()[1].id,
            scale: 2.0,
            angle: 37.0,
            ..Default::default()
        },
    };
    s.input.sequences[0].slots[0] = drawer_core::tip_texture::Slot {
        flip: [true, true],
        texture: Some(texture),
    };
    s.input.variations[0].dual.controls.texture = Some(texture);
    s.input.variations[0].dual.controls.flip = [false, true];
    s.input.materials[0].edge = 0.7;
    let preset =
        drawer_core::brush_presets::Preset::capture(&s, BrushTip::Pen, "Per-tip textures")?;
    if drawer_core::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        != preset
    {
        return Err("Tip texture preset round trip lost controls".into());
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
    if restarted.workspaces.store.current.input.sequences[0].slots[0].texture != Some(texture)
        || restarted.workspaces.store.current.input.variations[0]
            .dual
            .controls
            .texture
            != Some(texture)
        || restarted.workspaces.store.current.input.materials[0].edge != 0.7
        || *restarted.materials.library != *library
    {
        return Err("Tip texture / edge / material settings did not survive restart".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_tip_texture_views_focus_escape_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
