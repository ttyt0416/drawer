use super::editor::{Editor, button, check};
use crate::egui;
use drawer_core::{
    assets::BrushTip,
    pointer_input::{Event, Kind, Phase, Source},
};
use std::path::Path;
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut cases = vec![];
    for tool in BrushTip::ALL {
        for water in [false, true] {
            for ending in [
                "outside-up",
                "focus-loss",
                "pointer-cancel",
                "escape",
                "pen-device-cancel",
            ] {
                let mut e = Editor::new(1.25)?;
                e.app.set_tool(tool);
                e.app.panels.settings.brush_size = 5.;
                if tool == BrushTip::Eraser {
                    e.app.documents[0].layers[0].pixels.fill([255; 4]);
                }
                if tool == BrushTip::Smudge {
                    for (n, p) in e.app.documents[0].layers[0].pixels.iter_mut().enumerate() {
                        *p = if n % 64 < 32 {
                            [20, 40, 160, 255]
                        } else {
                            [220, 80, 20, 255]
                        };
                    }
                }
                if water {
                    e.app.workspaces.store.current.input.materials[tool as usize] =
                        drawer_core::brush_material::Settings {
                            mode: drawer_core::brush_material::Mode::Water,
                            edge: 1.,
                            water: 0.,
                            mixing: 0.,
                            retention: 0.,
                            supply: 1.,
                            ..Default::default()
                        };
                }
                let source = if ending == "pen-device-cancel" {
                    Source {
                        kind: Kind::Pen,
                        device: 289,
                        contact: 1,
                    }
                } else {
                    Source::MOUSE
                };
                let event = |phase, pos: egui::Pos2| Event {
                    source,
                    phase,
                    position: [f64::from(pos.x), f64::from(pos.y)],
                    pressure: Some(1.),
                    attributes: Default::default(),
                    origin: Default::default(),
                };
                let before = e.app.documents[0].snapshot();
                let history = e.app.documents[0].history_counts();
                let p = e.point([10.5, 20.5])?;
                let q = e.point([45.5, 20.5])?;
                if ending == "pen-device-cancel" {
                    e.app
                        .drawing
                        .virtual_events(vec![event(Phase::Down, p), event(Phase::Move, q)], true);
                    e.frame(vec![]);
                } else {
                    e.frame(vec![
                        egui::Event::PointerMoved(p),
                        button(p, true),
                        egui::Event::PointerMoved(q),
                    ]);
                }
                check(
                    e.app.drawing.pending(),
                    "Acceptance contact did not remain pending",
                )?;
                check(
                    e.app.documents[0].snapshot() == before,
                    "Pending stroke altered committed pixels",
                )?;
                match ending {
                    "outside-up" => e.frame(vec![button(egui::pos2(1200., q.y), false)]),
                    "focus-loss" => e.frame(vec![egui::Event::WindowFocused(false)]),
                    "pointer-cancel" => e.frame(vec![egui::Event::PointerGone]),
                    "escape" => e.frame(vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }]),
                    _ => {
                        e.app
                            .drawing
                            .virtual_events(vec![event(Phase::Cancel, q)], true);
                        e.frame(vec![]);
                    }
                }
                check(
                    !e.app.drawing.pending(),
                    "Ending retained an unfinished contact",
                )?;
                if ending == "outside-up" {
                    let d = &mut e.app.documents[0];
                    check(
                        d.history_counts().0 == history.0 + 1,
                        "Outside Up did not commit once",
                    )?;
                    let after = d.snapshot();
                    check(
                        after != before
                            && d.undo()
                            && d.snapshot() == before
                            && d.redo()
                            && d.snapshot() == after,
                        "Outside release history or pixels differ",
                    )?;
                } else {
                    check(
                        e.app.documents[0].snapshot() == before
                            && e.app.documents[0].history_counts() == history,
                        "Forced interruption committed part of the stroke",
                    )?;
                    check(
                        !e.app.drawing.cancellation_reasons.is_empty(),
                        "Cancellation has no local diagnostic cause",
                    )?;
                    e.frame(vec![
                        egui::Event::WindowFocused(true),
                        egui::Event::PointerMoved(q),
                        button(q, false),
                    ]);
                    check(
                        e.app.documents[0].snapshot() == before && !e.app.drawing.pending(),
                        "Held movement/release restarted an interrupted stroke",
                    )?;
                    e.frame(vec![
                        egui::Event::PointerMoved(p),
                        button(p, true),
                        egui::Event::PointerMoved(q),
                        button(q, false),
                    ]);
                    check(
                        e.app.documents[0].history_counts().0 == history.0 + 1,
                        "Fresh press after interruption did not recover",
                    )?;
                }
                cases.push(serde_json::json!({"tool":format!("{tool:?}"),"waterEdge":water,"ending":ending,"passed":true}));
            }
        }
    }
    // The production popup renderer tests texture views, cancel, settings restart,
    // and an edge-enabled watercolor release/focus-loss/Escape separately.
    crate::tip_texture::smoke(root)?;
    let mut e = Editor::new(1.)?;
    let storage = drawer_core::storage::UserStorage::at(root.join("cancel-profile"))
        .map_err(|e| e.to_string())?;
    e.app = e.app.with_storage(storage.clone());
    e.frame(vec![]);
    e.frame(vec![]);
    let p = e.point([20.5, 20.5])?;
    e.frame(vec![egui::Event::PointerMoved(p), button(p, true)]);
    e.frame(vec![egui::Event::WindowFocused(false)]);
    let path = storage.root().join("settings/input-cancellations.json");
    let log: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    check(
        log["events"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["reason"] == "focus-loss")),
        "Production cancellation was not saved locally",
    )?;
    Ok(
        serde_json::json!({"scenarios":cases,"popupTexturesAndSettingsRestart":true,"localDiagnosticBound":32,"physicalCaptureDriverValidated":false}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn outside_release_all_tools_water_edges_cancel_restore_fresh_press_and_local_log() {
        let r = crate::tests::TestDirectory::new();
        super::run(&r.0).unwrap();
    }
}
