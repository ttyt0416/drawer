use super::*;
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::transform_policy_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Box::new(Drawer::new(&ctx, None));
    app.simple_view = true;
    app.create_canvas(CanvasSpec {
        width: 16,
        height: 16,
        dpi: 300.,
        background: Background::Transparent,
    })
    .map_err(|e| e.to_string())?;
    app.documents[0].commit_state(drawer_core::deform_verify::fixture()?.snapshot())?;
    let original = app.documents[0].snapshot();
    let counts = app.documents[0].history_counts();
    let mut time = 0.;
    let frame =
        |app: &mut Drawer, time: &mut f64, mut events: Vec<egui::Event>| -> Result<(), String> {
            *time += 0.1;
            let modifiers = events
                .iter()
                .rev()
                .find_map(|e| match e {
                    egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            crate::input_replay::frame(&ctx, app, *time, events);
            app.wait_transform_preview(&ctx, time)
        };
    let pointer = |p: egui::Pos2, pressed, shift| {
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers {
                    shift,
                    ..Default::default()
                },
            },
        ]
    };
    let click = |app: &mut Drawer, time: &mut f64, p: egui::Pos2, shift| -> Result<(), String> {
        for down in [true, false] {
            frame(app, time, pointer(p, down, shift))?;
        }
        Ok(())
    };
    app.execute_command(&ctx, "mesh-puppet");
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![])?;
    }
    let pad = app.mesh_deform.pad.ok_or("Mesh pad missing")?;
    click(&mut app, &mut time, pad.center(), false)?;
    click(
        &mut app,
        &mut time,
        egui::pos2(pad.center().x, pad.min.y + 2.),
        true,
    )?;
    let e = app.mesh_deform.draft.as_ref().ok_or("Mesh closed")?;
    if e.chosen.len() != 2 || !e.chosen.contains(&1) || !e.chosen.contains(&4) {
        return Err(format!("Shift-click mesh set selection: {:?}", e.chosen));
    }
    let before = e.mesh.clone();
    let start = pad.center() + egui::vec2(2., 1.);
    let end = start + egui::vec2(8., 3.);
    frame(&mut app, &mut time, pointer(start, true, false))?;
    frame(&mut app, &mut time, vec![egui::Event::PointerMoved(end)])?;
    frame(&mut app, &mut time, pointer(end, false, false))?;
    let e = app.mesh_deform.draft.as_ref().unwrap();
    for n in 0..9 {
        if e.chosen.contains(&n) {
            if e.mesh.points[n] == before.points[n] {
                return Err("Selected mesh point did not move".into());
            }
        } else if e.mesh.points[n] != before.points[n] {
            return Err("Mesh drag moved an unselected point".into());
        }
    }
    if e.prepared.is_none() || app.documents[0].snapshot() != original {
        return Err(format!("Group mesh drag preview/source: {:?}", app.status));
    }
    app.apply_mesh_deform()?;
    let first = app.documents[0].snapshot();
    let native = root.join("mesh-set-ui.drawer");
    app.save_to(0, &native)?;
    if app.documents[0].history_counts().0 != counts.0 + 1 {
        return Err("Mesh set did not use one Undo".into());
    }
    app.execute_command(&ctx, "mesh-puppet");
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![])?;
    }
    let resume = app
        .mesh_deform
        .resume
        .ok_or("Resume saved mesh button missing")?;
    click(&mut app, &mut time, resume.center(), false)?;
    let e = app.mesh_deform.draft.as_ref().unwrap();
    if !e.resumed
        || e.mesh != first.scene.mesh_edit.as_ref().unwrap().settings.mesh
        || e.chosen.len() != 2
    {
        return Err("Saved mesh UI did not restore points/selection".into());
    }
    app.preview_mesh_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    app.apply_mesh_deform()?;
    if app.documents[0].snapshot() != first || app.documents[0].history_counts().0 != counts.0 + 1 {
        return Err("Unchanged resumed mesh was applied twice".into());
    }
    app.execute_command(&ctx, "mesh-puppet");
    app.resume_mesh_deform(&ctx)?;
    app.mesh_deform.draft.as_mut().unwrap().mesh.points[4][0] += 0.02;
    app.preview_mesh_deform(&ctx)?;
    app.wait_transform_preview(&ctx, &mut time)?;
    frame(
        &mut app,
        &mut time,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )?;
    if app.mesh_deform.draft.is_some() || app.documents[0].snapshot() != first {
        return Err("Cancelled resumed mesh changed saved state".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != original {
        return Err("Mesh set Undo did not restore original pixels/state".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn mesh_multiple_pointer_selection_drag_resume_cancel_undo() {
        let root = crate::tests::TestDirectory::new();
        super::smoke(&root.0).unwrap();
    }
}
