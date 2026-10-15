use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};
fn sequence() -> drawer_core::input_replay::Sequence {
    let mut s = drawer_core::input_replay::Sequence::default();
    s.mouse_pressure[0].enabled = true;
    s.mouse_pressure[0].seed = 19;
    s.pen_backend_available = false;
    for sample in &mut s.samples {
        sample.event.source = drawer_core::pointer_input::Source::MOUSE;
        sample.event.pressure = None;
    }
    s
}
pub(super) fn smoke() -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut s = sequence();
    let original = super::input_replay::run(&s, &cancel, &progress)?;
    if !original.record.passed {
        return Err("Mouse taper replay/undo failed".into());
    }
    s.cursors.tools[0].prediction = !s.cursors.tools[0].prediction;
    let prediction = super::input_replay::run(&s, &cancel, &progress)?;
    if !prediction.record.passed || prediction.pixels != original.pixels {
        return Err("Prediction display changed committed mouse pixels".into());
    }
    s.mouse_pressure[0].speed = true;
    let speed = super::input_replay::run(&s, &cancel, &progress)?;
    if !speed.record.passed || speed.pixels == original.pixels {
        return Err("Mouse speed factor was not independent".into());
    }
    let mut missing = sequence();
    missing.samples.pop();
    missing.expected_strokes = 0;
    let missing = super::input_replay::run(&missing, &cancel, &progress)?;
    if !missing.record.passed
        || !missing.record.incomplete_cancelled
        || missing.pixels.iter().any(|p| *p != [0; 4])
    {
        return Err("Missing mouse release committed partial reconstruction".into());
    }
    let mut spray = sequence();
    spray.mouse_pressure[0].effect = drawer_core::mouse_pressure::Effect::Spray;
    for (n, sample) in spray.samples.iter_mut().enumerate() {
        sample.event.position = [32.5, 20.5];
        sample.micros = n as u64 * 50000;
    }
    let first = super::input_replay::run(&spray, &cancel, &progress)?;
    let second = super::input_replay::run(&spray, &cancel, &progress)?;
    if !first.record.passed || !second.record.passed || first.pixels != second.pixels {
        return Err("Stationary mouse spray was not reproducible".into());
    }
    Ok(())
}
pub(super) fn settings_ui(ui: &mut egui::Ui, settings: &mut drawer_core::mouse_pressure::Settings) {
    use drawer_core::mouse_pressure::Effect;
    egui::ComboBox::from_id_salt("mouse-material")
        .selected_text(format!("{:?}", settings.effect))
        .show_ui(ui, |ui| {
            for (effect, label) in [
                (Effect::Normal, "Normal"),
                (Effect::Blend, "Blend with layer"),
                (Effect::Smudge, "Smudge layer pixels"),
                (Effect::Spray, "Timed spray"),
                (Effect::Wet, "Wet diffusion"),
            ] {
                ui.selectable_value(&mut settings.effect, effect, label);
            }
        });
    if [Effect::Blend, Effect::Smudge, Effect::Wet].contains(&settings.effect) {
        super::edit::slider(
            ui,
            &mut settings.strength,
            0.0..=1.0,
            "Layer pickup / diffusion strength",
            "mouse-pressure",
        );
    }
    if settings.effect == Effect::Spray {
        ui.horizontal_wrapped(|ui| {
            ui.label("Fixed spray period");
            let mut ms = settings.spray_period_micros as f64 / 1000.0;
            if super::edit::number(ui, &mut ms, 1.0..=1000.0, 1.0, " ms", "mouse-pressure")
                .changed()
            {
                settings.spray_period_micros = (ms * 1000.0).round() as u32;
            }
            ui.label("Particles per tick");
            super::edit::number(
                ui,
                &mut settings.spray_dabs,
                1.0..=64.0,
                1.0,
                "",
                "mouse-pressure",
            );
        });
    }
    ui.checkbox(
        &mut settings.enabled,
        "Distance-based start/end mouse pressure",
    );
    ui.horizontal_wrapped(|ui| {
        ui.label("Start length (document px)");
        super::edit::number(
            ui,
            &mut settings.start_length,
            0.0..=1e6,
            1.0,
            "",
            "mouse-pressure",
        );
        ui.label("End length");
        super::edit::number(
            ui,
            &mut settings.end_length,
            0.0..=1e6,
            1.0,
            "",
            "mouse-pressure",
        );
    });
    for (label, curve) in [
        ("Start curve", &mut settings.start_curve),
        ("End curve", &mut settings.end_curve),
    ] {
        ui.collapsing(label, |ui| {
            ui.label("Fixed endpoints 0 → 1; output at 25%, 50%, 75% of taper length");
            for n in 1..4 {
                let range = curve[n - 1]..=curve[n + 1];
                super::edit::slider(
                    ui,
                    &mut curve[n],
                    range,
                    format!("{}%", n * 25),
                    "mouse-pressure",
                );
            }
        });
    }
    ui.checkbox(&mut settings.speed, "Multiply by mouse speed response");
    ui.horizontal_wrapped(|ui| {
        ui.label("Fast speed (document px/s)");
        super::edit::number(
            ui,
            &mut settings.speed_full,
            0.01..=1e6,
            10.0,
            "",
            "mouse-pressure",
        );
    });
    super::edit::slider(
        ui,
        &mut settings.speed_minimum,
        0.0..=1.0,
        "Fast-motion pressure factor",
        "mouse-pressure",
    );
    ui.horizontal_wrapped(|ui| {
        ui.label("Pattern seed");
        super::edit::number(
            ui,
            &mut settings.seed,
            0.0..=u32::MAX as f64,
            1.0,
            "",
            "mouse-pressure",
        );
        if ui.button("Next pattern seed").clicked() {
            settings.seed = settings.seed.wrapping_add(1);
        }
    });
    ui.label("Start/end overlap multiplies the curves. A stationary dot uses pressure 1. Missing or nonincreasing clocks use speed factor 1. Current pressure size/opacity targets apply. End taper is reconstructed from original pixels on release; predictions remain display-only.");
    ui.label("Layer pickup reads this stroke's earlier dabs. Wet diffusion runs once on release. Timed spray records elapsed app time while held. Pattern seed and stroke behavior also work independently of taper.");
}
#[cfg(test)]
mod tests {
    use super::super::{
        input_replay::frame,
        tests::{TestDirectory, tiny_spec},
    };
    use super::*;
    use drawer_core::pointer_input::{Attributes, Event, Phase, Source};
    fn setup() -> (egui::Context, Drawer) {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(CanvasSpec {
            width: 64,
            height: 48,
            ..tiny_spec()
        })
        .unwrap();
        app.panels.settings.brush_size = 8.0;
        app.workspaces.store.current.input.mouse_pressure[0].enabled = true;
        app.workspaces.store.current.input.mouse_pressure[0].seed = 37;
        app.workspaces.store.current.pressure.tools[0].opacity = true;
        frame(&ctx, &mut app, 0.0, vec![]);
        frame(&ctx, &mut app, 0.1, vec![]);
        (ctx, app)
    }
    fn event(
        ctx: &egui::Context,
        app: &Drawer,
        point: [f64; 2],
        phase: Phase,
        micros: u64,
    ) -> Event {
        let d = &app.documents[0];
        let r = app.canvas_view.viewport_rect().unwrap();
        let position = d.view.document_to_screen(
            point,
            [64.0, 48.0],
            [r.center().x as f64, r.center().y as f64],
            ctx.pixels_per_point() as f64,
        );
        Event {
            source: Source::MOUSE,
            phase,
            position,
            pressure: None,
            attributes: Attributes {
                time_micros: Some(micros),
                ..Default::default()
            },
            origin: Default::default(),
        }
    }
    #[test]
    fn stationary_frame_clock_previews_then_commits_the_same_fixed_time_dabs() {
        let (ctx, mut app) = setup();
        app.workspaces.store.current.input.mouse_pressure[0].effect =
            drawer_core::mouse_pressure::Effect::Spray;
        app.workspaces.store.current.input.mouse_pressure[0].spray_dabs = 3;
        let before = app.documents[0].snapshot();
        let down = event(&ctx, &app, [32.5, 24.5], Phase::Down, 0);
        app.drawing.virtual_events(vec![down], false);
        frame(&ctx, &mut app, 1.0, vec![]);
        let initial = app
            .drawing
            .preview(app.documents[0].id)
            .unwrap()
            .dab_count();
        assert_eq!(initial, 3);
        frame(&ctx, &mut app, 1.1, vec![]);
        assert_eq!(
            app.drawing
                .preview(app.documents[0].id)
                .unwrap()
                .dab_count(),
            18
        );
        frame(&ctx, &mut app, 1.2, vec![]);
        assert_eq!(
            app.drawing
                .preview(app.documents[0].id)
                .unwrap()
                .dab_count(),
            33
        );
        assert!(app.documents[0].snapshot() == before);
        let up = event(&ctx, &app, [32.5, 24.5], Phase::Up, 100000);
        app.drawing.virtual_events(vec![up], false);
        frame(&ctx, &mut app, 1.2, vec![]);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        let meta = app.documents[0].scene.mouse_strokes[0].clone();
        let mut direct = Document::create(
            CanvasSpec {
                width: 64,
                height: 48,
                ..tiny_spec()
            },
            16384,
        )
        .unwrap();
        let mut replay =
            drawer_core::mouse_pressure::Replay::begin(&direct, 0, meta.style, meta.settings)
                .unwrap();
        for micros in [0, 100000] {
            replay
                .push(drawer_core::mouse_pressure::Sample {
                    point: [32.5, 24.5],
                    micros: Some(micros),
                })
                .unwrap();
        }
        replay
            .render(&direct, true)
            .unwrap()
            .commit_mouse(&mut direct, replay.metadata())
            .unwrap();
        assert_eq!(direct.layers[0].pixels, app.documents[0].layers[0].pixels);
    }
    #[test]
    fn cancelling_during_computation_never_publishes_partial_pixels_or_metadata() {
        let (ctx, mut app) = setup();
        let before = app.documents[0].snapshot();
        app.drawing.cancel_check = Some(Box::new(|| true));
        let events = vec![
            event(&ctx, &app, [8.5, 20.5], Phase::Down, 0),
            event(&ctx, &app, [52.5, 20.5], Phase::Up, 10000),
        ];
        app.drawing.virtual_events(events, false);
        frame(&ctx, &mut app, 0.2, vec![]);
        assert!(!app.drawing.pending());
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert!(app.status.as_ref().unwrap().contains("cancelled"));
    }
    #[test]
    fn prediction_replay_speed_and_missing_release_use_real_pixel_undo_path() {
        smoke().unwrap();
    }
    #[test]
    fn preview_is_temporary_settings_are_frozen_and_native_metadata_reopens_with_pixels() {
        let (ctx, mut app) = setup();
        let before = app.documents[0].snapshot();
        let settings = app.workspaces.store.current.input.mouse_pressure[0];
        let events = vec![
            event(&ctx, &app, [8.5, 20.5], Phase::Down, 0),
            event(&ctx, &app, [32.5, 20.5], Phase::Move, 10000),
        ];
        app.drawing.virtual_events(events, false);
        frame(&ctx, &mut app, 0.2, vec![]);
        assert!(app.drawing.pending());
        assert!(app.drawing.changed(app.documents[0].id));
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        app.workspaces.store.current.input.mouse_pressure[0].enabled = false;
        app.workspaces.store.current.input.mouse_pressure[0].seed = 99;
        let up = event(&ctx, &app, [52.5, 20.5], Phase::Up, 20000);
        app.drawing.virtual_events(vec![up], false);
        frame(&ctx, &mut app, 0.3, vec![]);
        assert_eq!(app.documents[0].scene.mouse_strokes[0].settings, settings);
        assert_eq!(app.documents[0].history_counts(), (1, 0));
        assert!(!app.drawing.pending());
        let after = app.documents[0].snapshot();
        app.documents[0].undo();
        assert!(app.documents[0].snapshot() == before);
        app.documents[0].redo();
        assert!(app.documents[0].snapshot() == after);
        let root = TestDirectory::new();
        let path = root.0.join("mouse.drawer");
        drawer_core::files::save(&mut app.documents[0], &path).unwrap();
        let read = drawer_core::files::open(&path, drawer_core::document::DEFAULT_DOCUMENT_BUDGET)
            .unwrap();
        assert!(read.snapshot() == after);
    }
    #[test]
    fn escape_and_reconstruction_budget_failure_discard_every_pending_pixel() {
        let (ctx, mut app) = setup();
        let before = app.documents[0].snapshot();
        let down = event(&ctx, &app, [8.5, 20.5], Phase::Down, 0);
        app.drawing.virtual_events(vec![down], false);
        frame(&ctx, &mut app, 0.2, vec![]);
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(&ctx, &mut app, 0.3, vec![escape]);
        assert!(!app.drawing.pending());
        assert!(app.documents[0].snapshot() == before);
        let events = vec![
            event(&ctx, &app, [8.5, 20.5], Phase::Down, 10000),
            event(&ctx, &app, [1e7, 20.5], Phase::Up, 20000),
        ];
        app.drawing.virtual_events(events, false);
        frame(&ctx, &mut app, 0.4, vec![]);
        assert!(!app.drawing.pending());
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
        assert!(app.status.as_ref().unwrap().contains("budget"));
    }
    #[test]
    fn off_is_full_pressure_while_local_settings_and_input_records_preserve_curves_times_seed() {
        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        let mut s = sequence();
        s.mouse_pressure[0].enabled = false;
        let first = super::super::input_replay::run(&s, &cancel, &progress).unwrap();
        for sample in &mut s.samples {
            sample.event.pressure = Some(0.0);
        }
        let second = super::super::input_replay::run(&s, &cancel, &progress).unwrap();
        assert_eq!(first.pixels, second.pixels);
        assert!(first.record.passed && second.record.passed);
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.workspaces.store.current.input.mouse_pressure[2] =
            drawer_core::mouse_pressure::Settings {
                enabled: true,
                seed: 12345,
                start_length: 7.5,
                end_length: 22.0,
                start_curve: [0.0, 0.1, 0.3, 0.8, 1.0],
                speed: true,
                ..Default::default()
            };
        app.persist_workspace().unwrap();
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(
            restarted.workspaces.store.current.input.mouse_pressure,
            app.workspaces.store.current.input.mouse_pressure
        );
        let path = root.0.join("replay.json");
        let original = sequence();
        original.save(&path).unwrap();
        let read = drawer_core::input_replay::Sequence::read(&path).unwrap();
        assert_eq!(read, original);
        assert_eq!(read.mouse_pressure[0].seed, 19);
        assert_eq!(read.samples[2].micros, 2000);
    }
}
