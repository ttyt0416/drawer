use super::*;
use drawer_core::{
    bucket::{Expansion, Reference, Settings},
    fill_reference::{self as refs, Filter},
    pointer_input::{Kind, Phase, Source},
};
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::fill_reference_verify::verify(root)?;
    for pen in [false, true] {
        for case in 0..10 {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            let mut time = 0.0;
            let mut d = drawer_core::fill_reference_verify::fixture()?;
            d.view.angle = 90.0;
            d.view.zoom = 8.0;
            if case == 9 {
                d.layers[0].pixels = d.layers[1].pixels.clone();
                d.layers[1].pixels.fill([0; 4]);
            }
            app.add_document(d);
            app.choose_color([220, 40, 60, 128]);
            let mut s = Settings {
                reference: match case {
                    0 => Reference::Marked,
                    1 => Reference::Selected,
                    2 => Reference::Folder(0),
                    _ => Reference::Composite,
                },
                antialias: false,
                ..Default::default()
            };
            for n in 1..5 {
                refs::select(&mut s.selected, n, true);
            }
            s.exclude = match case {
                4 => Filter {
                    text: true,
                    ..Default::default()
                },
                5 => Filter {
                    balloon: true,
                    ..Default::default()
                },
                6 => Filter {
                    draft: true,
                    ..Default::default()
                },
                7 => Filter {
                    paper: true,
                    ..Default::default()
                },
                8 => Filter {
                    locked: true,
                    ..Default::default()
                },
                9 => Filter {
                    editing: true,
                    ..Default::default()
                },
                _ => Default::default(),
            };
            app.workspaces.store.current.input.fills[0] = s;
            let step = |app: &mut Drawer, time: &mut f64, events| {
                *time += 0.1;
                super::input_replay::frame(&ctx, app, *time, events);
            };
            step(&mut app, &mut time, vec![]);
            step(&mut app, &mut time, vec![]);
            app.execute_command(&ctx, "bucket-tool");
            let path = root.join(format!("fill-refs-{pen}-{case}.drawer"));
            drawer_core::files::save(&mut app.documents[0], &path).map_err(|e| e.to_string())?;
            let before = app.documents[0].snapshot();
            let history = app.documents[0].history_counts();
            let pointer = |app: &Drawer, phase| {
                let mut e = super::precision::pointer(&ctx, app, [3.5; 2], phase);
                if pen {
                    e.source = Source {
                        kind: Kind::Pen,
                        device: 101,
                        contact: 1,
                    };
                    e.pressure = Some(0.5);
                }
                e
            };
            app.drawing
                .virtual_events(vec![pointer(&app, Phase::Down)], pen);
            step(&mut app, &mut time, vec![]);
            if !app.drawing.pending()
                || app.documents[0].snapshot() != before
                || app.documents[0].dirty
                || app.drawing.bucket_preview(app.documents[0].id).is_none()
            {
                return Err(format!(
                    "Fill reference draft {pen}/{case}: {:?}",
                    app.status
                ));
            }
            app.drawing
                .virtual_events(vec![pointer(&app, Phase::Up)], pen);
            step(&mut app, &mut time, vec![]);
            if app.drawing.pending()
                || app.documents[0].history_counts().0 != history.0 + 1
                || app.documents[0].layers[0].pixel(7, 7, 3, 3) != [220, 40, 60, 128]
                || app.documents[0].layers[0].pixel(7, 7, 0, 0)[3]
                    != if case >= 4 { 128 } else { 0 }
                || app.documents[0].scene != before.scene
            {
                return Err(format!(
                    "Fill reference production {pen}/{case}: {:?}",
                    app.status
                ));
            }
            let after = app.documents[0].snapshot();
            if !app.documents[0].undo()
                || app.documents[0].snapshot() != before
                || !app.documents[0].redo()
                || app.documents[0].snapshot() != after
            {
                return Err("Fill reference production Undo/Redo".into());
            }
            app.documents[0].undo();
            app.drawing
                .virtual_events(vec![pointer(&app, Phase::Down)], pen);
            step(&mut app, &mut time, vec![]);
            app.workspaces.store.current.input.fills[0].exclude.draft = !s.exclude.draft;
            app.drawing
                .virtual_events(vec![pointer(&app, Phase::Up)], pen);
            step(&mut app, &mut time, vec![]);
            if app.drawing.pending() || app.documents[0].snapshot() != before {
                return Err("Fill filter change committed a stale preview".into());
            }
            app.workspaces.store.current.input.fills[0] = Settings {
                expansion: Expansion::Darkest,
                fill_narrow: true,
                expand: 2,
                ..s
            };
            let preset = drawer_core::brush_presets::Preset::capture(
                &app.workspaces.store.current,
                BrushTip::Pen,
                "Fill references",
            )?;
            let decoded = drawer_core::brush_presets::Preset::decode(
                &preset.encode().map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if decoded.fill != app.workspaces.store.current.input.fills[0] {
                return Err("Fill reference preset roundtrip".into());
            }
            let settings_path = root.join("fill-reference-workspace.json");
            app.workspaces
                .store
                .current
                .export(&settings_path)
                .map_err(|e| e.to_string())?;
            if drawer_core::workspaces::Snapshot::read(&settings_path)
                .map_err(|e| e.to_string())?
                .input
                .fills
                != app.workspaces.store.current.input.fills
            {
                return Err("Fill reference settings roundtrip".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn production_reference_sets_filters_mouse_pen_cancel_and_presets() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
