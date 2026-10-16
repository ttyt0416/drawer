use super::*;
use drawer_core::{
    brush_presets::Preset, input_replay::Sequence, material_library::Library,
    user_tools::Library as Tools,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize},
};
fn replay(p: &Preset) -> Sequence {
    let mut s = Sequence {
        size: p.values[0],
        aspect: p.values[1],
        angle: p.values[2],
        ..Default::default()
    };
    s.pressure.tools[0] = p.pressure;
    s.sequences[0] = p.sequence;
    s.profiles[0] = p.profile;
    s.textures[0] = p.overlay;
    s.materials[0] = p.material;
    for sample in &mut s.samples {
        sample.event.pressure = Some(1.0);
    }
    s.expected_strokes = usize::from(p.name != "Empty fibres");
    s
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    let (tools, materials) = drawer_core::tool_bundle::verify(root)?;
    let bundle = root.join("third-offline-pc");
    drawer_core::tool_bundle::export(&bundle, &tools, &materials).map_err(|e| e.to_string())?;
    let (third, third_materials) =
        drawer_core::tool_bundle::import(&bundle, &Tools::default(), &Library::default())
            .map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    for (a, b) in tools.tools.iter().zip(&third.tools) {
        let first = super::input_replay::run_with_materials(
            &replay(&a.preset),
            Arc::new(materials.clone()),
            &cancel,
            &progress,
        )?;
        let second = super::input_replay::run_with_materials(
            &replay(&b.preset),
            Arc::new(third_materials.clone()),
            &cancel,
            &progress,
        )?;
        if !first.record.passed || !second.record.passed || first.pixels != second.pixels {
            return Err(format!(
                "Portable production drawing changed: {}",
                a.preset.name
            ));
        }
    }
    let user = root.join("slot-user");
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None)
        .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
    app.create_canvas(CanvasSpec {
        width: 8,
        height: 8,
        dpi: 300.0,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.publish_materials(materials.clone())?;
    app.sync_tool_properties();
    let mut next = app.capture_workspace();
    next.user_tools = tools.clone();
    app.stage_workspace(next, app.workspaces.store.active.clone());
    app.apply_workspace(
        drawer_core::workspaces::Scope {
            other: true,
            ..Default::default()
        },
        None,
    )?;
    let before = app.documents[0].snapshot();
    let id = tools.tools[0].id;
    app.select_user_tool(id)?;
    let active = app.capture_workspace();
    let mut deleted = active.user_tools.clone();
    deleted.delete_tools(&[id])?;
    let mut history = drawer_core::user_tools::History::default();
    history.record(active.user_tools.clone(), &deleted);
    let removed = deleted.apply_to(&active)?;
    if removed.user_tools.active.is_some() {
        return Err("Active slot deletion did not select a safe base".into());
    }
    if !history.undo(&mut deleted) {
        return Err("Slot deletion undo unavailable".into());
    }
    let restored = deleted.apply_to(&removed)?;
    if restored.user_tools.active != Some(id)
        || restored.user_tools.entry(id) != active.user_tools.entry(id)
        || *app.materials.library != materials
        || app.documents[0].snapshot() != before
        || app.documents[0].history_counts() != (0, 0)
    {
        return Err("List undo changed picture/material or lost active slot".into());
    }
    let restarted = Drawer::new(&egui::Context::default(), None)
        .with_storage(UserStorage::at(user).map_err(|e| e.to_string())?);
    if restarted
        .workspaces
        .store
        .current
        .user_tools
        .tools
        .iter()
        .map(|t| t.id)
        .collect::<Vec<_>>()
        != tools.tools.iter().map(|t| t.id).collect::<Vec<_>>()
        || *restarted.materials.library != materials
    {
        return Err("Slot order/material restart failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn portable_fibres_production_drawing_and_restart() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
