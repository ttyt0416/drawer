use super::*;
/// Exercise all three new tools through the production event/replay path.
pub(super) fn smoke() -> Result<(), String> {
    use drawer_core::{input_replay::Sequence, pointer_input::Source};
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    drawer_core::extended_brushes::verify()?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for tip in [BrushTip::Marker, BrushTip::Smudge, BrushTip::PixelPen] {
        for mouse in [false, true] {
            let n = BrushTip::ALL.iter().position(|t| *t == tip).unwrap();
            let mut s = Sequence {
                tool: tip,
                ..Default::default()
            };
            s.pressure.tools[n].size = false;
            s.pressure.tools[n].opacity = false;
            for sample in &mut s.samples {
                sample.event.pressure = Some(1.0);
                if mouse {
                    sample.event.source = Source::MOUSE;
                    sample.event.pressure = None;
                }
            }
            if tip == BrushTip::Smudge {
                s.background = [210, 50, 10, 120];
                s.color = [0, 255, 0, 0];
                s.materials[n].dilution = 0.5;
                s.materials[n].water = 0.5;
            }
            if tip == BrushTip::PixelPen {
                s.size = 1.0;
                s.color = [20, 80, 220, 128];
            }
            let a = super::input_replay::run(&s, &cancel, &progress)?;
            let b = super::input_replay::run(&s, &cancel, &progress)?;
            if !a.record.passed || a.pixels != b.pixels {
                return Err(format!(
                    "{} mouse/pen replay or single-undo mismatch",
                    tip.label()
                ));
            }
            if tip == BrushTip::PixelPen && a.pixels.iter().any(|p| p[3] != 0 && *p != s.color) {
                return Err("Pixel pen produced an antialiased edge".into());
            }
            if tip == BrushTip::Smudge && a.pixels.iter().any(|p| p[3] > 0 && p[1] > 60) {
                return Err("Smudge supplied green foreground".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, frame};
    #[test]
    fn all_new_tools_mouse_pen_native_replay_undo_and_binary_edges() {
        smoke().unwrap();
    }
    #[test]
    fn new_shortcuts_select_independent_properties_restart_and_scratchpad_stay_local() {
        use drawer_core::tool_properties::{Property, Tool};
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app =
            Drawer::new(&ctx, None).with_storage(UserStorage::at(storage.root().into()).unwrap());
        app.create_canvas(crate::tests::tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        for (key, tip, size) in [
            (egui::Key::Y, BrushTip::Marker, 12.0),
            (egui::Key::U, BrushTip::Smudge, 12.0),
            (egui::Key::P, BrushTip::PixelPen, 1.0),
        ] {
            for pressed in [true, false] {
                frame(
                    &ctx,
                    &mut app,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
            }
            assert_eq!(app.selected, tip);
            assert_eq!(app.panels.settings.brush_size, size);
        }
        app.set_quick_brush_size(2.0).unwrap();
        app.set_tool(BrushTip::Marker);
        assert_eq!(app.panels.settings.brush_size, 12.0);
        app.set_quick_brush_size(19.0).unwrap();
        app.set_tool(BrushTip::PixelPen);
        assert_eq!(app.panels.settings.brush_size, 2.0);
        app.persist_workspace().unwrap();
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().into()).unwrap());
        let props = restarted.workspaces.store.current.properties.unwrap();
        assert_eq!(props.tools[&Tool::Marker][&Property::Size].value, 19.0);
        assert_eq!(props.tools[&Tool::PixelPen][&Property::Size].value, 2.0);
        assert_eq!(props.tools[&Tool::Pen][&Property::Size].value, 12.0);
        let pad = &mut app.workspaces.store.current.scratchpad;
        pad.stroke(
            [2.5, 2.5],
            [12.5, 12.5],
            1.0,
            BrushTip::PixelPen,
            [20, 80, 220, 128],
        );
        assert!(
            pad.pixels
                .iter()
                .all(|p| p[3] == 0 || *p == [20, 80, 220, 128])
        );
        let blank = drawer_core::workspaces::Scratchpad::default();
        let mut smudge = blank.clone();
        smudge.stroke(
            [2.5, 2.5],
            [12.5, 12.5],
            8.0,
            BrushTip::Smudge,
            [0, 255, 0, 255],
        );
        assert_eq!(smudge, blank);
        assert!(app.documents[0].snapshot() == before);
        assert_eq!(app.documents[0].history_counts(), (0, 0));
    }
}
