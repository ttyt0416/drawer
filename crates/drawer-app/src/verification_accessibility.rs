use super::editor::{Editor, check};
use crate::*;
fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .into()
}
fn press(e: &mut Editor, k: egui::Key, mods: egui::Modifiers) {
    e.frame_modifiers(key(k, mods), mods);
    e.frame(vec![]);
}
fn luminance(c: egui::Color32) -> f64 {
    let p = c.to_array();
    [0, 1, 2]
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(i, w)| {
            let v = f64::from(p[i]) / 255.;
            w * if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
        .sum()
}
pub(super) fn run(root: &std::path::Path) -> Result<serde_json::Value, String> {
    let mut cases = Vec::new();
    for scale in [1., 2.] {
        let mut e = Editor::new(1.)?;
        e.app.interface_zoom(&e.ctx, scale)?;
        e.frame(vec![]);
        e.frame(vec![]);
        let before = e.app.documents[0].snapshot();
        let history = e.app.documents[0].history_counts();
        press(&mut e, egui::Key::Tab, egui::Modifiers::NONE);
        let previous = e.ctx.memory(|m| m.focused());
        press(&mut e, egui::Key::F10, egui::Modifiers::NONE);
        let search = crate::accessibility::query_id();
        check(
            e.app.keyboard.open && e.ctx.memory(|m| m.focused()) == Some(search),
            "F10 did not focus command search",
        )?;
        press(&mut e, egui::Key::Tab, egui::Modifiers::NONE);
        check(
            e.ctx.memory(|m| m.focused()).is_some_and(|id| id != search),
            "Tab did not leave command search",
        )?;
        press(&mut e, egui::Key::Tab, egui::Modifiers::SHIFT);
        check(
            e.ctx.memory(|m| m.focused()) == Some(search),
            "Shift+Tab did not return to command search",
        )?;
        press(&mut e, egui::Key::ArrowDown, egui::Modifiers::NONE);
        check(
            e.ctx.memory(|m| m.focused()) == Some(search),
            "Arrow navigation lost the search focus",
        )?;
        e.frame(vec![egui::Event::Text("@scale".into())]);
        e.frame(vec![]);
        check(
            e.app.keyboard.matches == ["@scale"],
            &format!(
                "Text input did not filter commands: scale={scale}, query={:?}, focus={:?}, matches={:?}",
                e.app.keyboard.query,
                e.ctx.memory(|m| m.focused()),
                e.app.keyboard.matches
            ),
        )?;
        let rect = e.app.keyboard.rect.ok_or("Command window missing")?;
        check(
            e.ctx.content_rect().expand(1.).contains_rect(rect),
            "Command window outside scaled content",
        )?;
        press(&mut e, egui::Key::Enter, egui::Modifiers::NONE);
        check(
            !e.app.keyboard.open && e.app.interface.open,
            "Enter did not open interface dialog",
        )?;
        press(&mut e, egui::Key::Escape, egui::Modifiers::NONE);
        check(
            !e.app.interface.open && e.ctx.memory(|m| m.focused()) == previous,
            "Dialog Escape failed to return focus",
        )?;
        press(&mut e, egui::Key::F10, egui::Modifiers::NONE);
        e.frame(vec![egui::Event::Text("@panels".into())]);
        e.frame(vec![]);
        press(&mut e, egui::Key::Enter, egui::Modifiers::NONE);
        check(e.app.panels.open, "Keyboard did not open panel controls")?;
        press(&mut e, egui::Key::Escape, egui::Modifiers::NONE);
        check(!e.app.panels.open, "Panel Escape did not close")?;
        press(&mut e, egui::Key::F10, egui::Modifiers::NONE);
        e.frame(vec![egui::Event::Text("@panels".into())]);
        e.frame(vec![]);
        let close = e
            .app
            .keyboard
            .close_id
            .ok_or("Missing keyboard close control")?;
        e.ctx.memory_mut(|m| m.request_focus(close));
        e.frame(vec![]);
        press(&mut e, egui::Key::Enter, egui::Modifiers::NONE);
        check(
            !e.app.keyboard.open && !e.app.panels.open,
            "Enter on Close executed the selected command",
        )?;
        e.app.interface_contrast(&e.ctx, true)?;
        e.frame(vec![]);
        let v = e.ctx.style_of(e.ctx.theme()).visuals.clone();
        let foreground = v
            .override_text_color
            .ok_or("Missing high-contrast foreground")?;
        let ratio = (luminance(foreground) + 0.05) / (luminance(v.panel_fill) + 0.05);
        check(
            ratio >= 7. && v.widgets.active.bg_stroke.width >= 2. && v.selection.stroke.width >= 2.,
            "High-contrast text or focus border failed",
        )?;
        press(&mut e, egui::Key::F10, egui::Modifiers::NONE);
        press(&mut e, egui::Key::F1, egui::Modifiers::NONE);
        check(
            e.app.workspaces.store.current.help.topic == "accessibility",
            "Focused command help did not resolve",
        )?;
        press(&mut e, egui::Key::Escape, egui::Modifiers::NONE);
        check(!e.app.keyboard.open, "Command Escape trapped focus")?;
        check(
            e.app.documents[0].snapshot() == before
                && e.app.documents[0].history_counts() == history
                && !e.app.documents[0].dirty,
            "Navigation changed drawing/history",
        )?;
        cases.push(serde_json::json!({"interfaceScale":scale,"tabShiftTab":true,"enterDialogs":true,"enterFocusedCloseDoesNotExecuteSelection":true,"escapeReturnsFocus":true,"f1ContextHelp":true,"textContrastRatio":ratio,"focusBorderPoints":v.widgets.active.bg_stroke.width,"documentUnchanged":true}));
    }
    let storage = UserStorage::at(root.join("accessibility-profile")).map_err(|e| e.to_string())?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None).with_storage(storage.clone());
    app.interface_contrast(&ctx, true)?;
    drop(app);
    let restarted = Drawer::new(&egui::Context::default(), None).with_storage(storage);
    check(
        restarted.interface.settings.high_contrast,
        "Contrast did not survive restart",
    )?;
    Ok(
        serde_json::json!({"mode":"production CPU egui keyboard events","cases":cases,"contrastRestart":true,"physicalAssistiveDeviceValidated":false}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn f10_tab_shift_tab_enter_escape_focus_help_contrast_scale_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
