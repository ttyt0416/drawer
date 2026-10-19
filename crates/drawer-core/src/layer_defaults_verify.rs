use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    layer_defaults::{self as core, Class, Data, Fields, Storage},
    layer_properties::{self as p, Target},
    layer_search::{Expression, Key, Kind},
};
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    d.layers[0].name = "PRIVATE SOURCE NAME".into();
    d.layers[0].pixels[2 * 8 + 2] = [180, 90, 40, 255];
    let mut scene = d.scene.clone();
    scene.selection = Some(crate::scene::Rect {
        min: [1.; 2],
        max: [4.; 2],
    });
    d.edit_scene(scene)?;
    let scope = crate::appearance::capture_scope(&d)?;
    let state = crate::appearance::prepare_expression(
        &d,
        Target::Raster(1),
        crate::appearance::Style {
            enabled: true,
            color: Some([10, 20, 30]),
            border: 1,
            scope: Some(scope),
            ..Default::default()
        },
        Some(Expression::Gray),
    )?;
    d.commit_state(state)?;
    let state = p::prepare_patch(
        &d,
        &[Target::Raster(1)],
        p::Patch {
            opacity: Some(42),
            blend: Some(crate::layer_blend::Mode::Multiply),
            locked: Some(true),
            pixels_locked: Some(true),
            position_locked: Some(true),
            color: Some(Some([1, 2, 3])),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    let before = d.snapshot();
    let history = d.history_counts();
    let key = Key::Layer(Target::Raster(1));
    let class = core::class(&d, key)?;
    let entry = core::capture(&d, key, Fields::supported(class))?;
    check(
        entry.appearance.scope.is_none()
            && entry.expression == Expression::Gray
            && entry.opacity == 42
            && entry.color == Some([1, 2, 3]),
        "Default capture properties/scope stripping",
    )?;
    let mut data = Data::default();
    data.set(entry.clone())?;
    let bytes = serde_json::to_vec(&data).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    for excluded in [
        "PRIVATE SOURCE NAME",
        "\"pixels\"",
        "\"id\"",
        "\"path\"",
        "\"rect\"",
        "\"mask\"",
    ] {
        check(
            !text.contains(excluded),
            "Layer defaults copied document content/identity",
        )?;
    }
    check(
        d.snapshot() == before && d.history_counts() == history,
        "Capturing defaults edited existing layer/history",
    )?;
    let mut created = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "New independent layer".into(),
            parent: None,
        },
    )?;
    core::apply_new(&d, &mut created.state, &data)?;
    d.commit_state(created.state)?;
    let new = p::properties(&d, Target::Raster(2));
    check(
        new.opacity == 42
            && new.blend == crate::layer_blend::Mode::Multiply
            && new.color == Some([1, 2, 3])
            && !new.locked
            && !new.pixels_locked
            && !new.position_locked
            && d.layers[1].name == "New independent layer"
            && d.layers[1].pixels == vec![[0; 4]]
            && d.scene.appearance.rasters[&2].scope.is_none()
            && crate::appearance::expression(&d, Target::Raster(2)) == Expression::Gray,
        "New same-kind layer defaults/content/locks",
    )?;
    check(
        d.layers[0] == before.layers[0]
            && p::properties(&d, Target::Raster(1)) == before.scene.layer_properties.rasters[&1]
            && d.scene.appearance.rasters[&1] == before.scene.appearance.rasters[&1],
        "Creation defaults changed existing layer",
    )?;
    check(
        d.undo() && d.snapshot() == before,
        "Layer creation/defaults not one Undo",
    )?;
    let mut vector = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Vector,
            name: "Different kind".into(),
            parent: None,
        },
    )?;
    core::apply_new(&d, &mut vector.state, &data)?;
    d.commit_state(vector.state)?;
    let id = d.scene.lines.layers[0].id;
    check(
        p::properties(&d, Target::Vector(id)).opacity == 100
            && d.scene.appearance.vectors.is_empty(),
        "Raster defaults leaked to vector layer",
    )?;
    let store = crate::storage::UserStorage::at(root.join("layer-default-settings"))
        .map_err(|e| e.to_string())?;
    let mut workspace = crate::workspaces::Store::default();
    workspace.current.layer_defaults = data.clone();
    workspace.save(&store).map_err(|e| e.to_string())?;
    let loaded = crate::workspaces::Store::load(&store)
        .map_err(|e| e.to_string())?
        .ok_or("Defaults settings missing")?;
    check(
        loaded.current.layer_defaults == data,
        "Defaults settings restart",
    )?;
    let mut blank = crate::layer_roles_verify::page()?;
    let pixels = blank.layers[0].pixels.clone();
    let dirty = blank.dirty;
    core::initialize(&mut blank, &loaded.current.layer_defaults)?;
    check(
        blank.history_counts() == (0, 0)
            && blank.dirty == dirty
            && blank.layers[0].pixels == pixels
            && p::properties(&blank, Target::Raster(1)).opacity == 42
            && crate::appearance::expression(&blank, Target::Raster(1)) == Expression::Gray,
        "New document defaults modified pixels/history",
    )?;
    let mut reset = loaded.current.layer_defaults;
    reset.reset(class);
    let mut reset_doc = crate::layer_roles_verify::page()?;
    core::initialize(&mut reset_doc, &reset)?;
    check(
        reset.is_default()
            && reset_doc.scene.appearance.is_default()
            && p::properties(&reset_doc, Target::Raster(1)).opacity == 100,
        "Reset layer-kind defaults",
    )?;
    let unsupported = Class {
        kind: Kind::Correction,
        storage: Storage::Raster,
    };
    check(
        !Fields::supported(unsupported).appearance && !Fields::supported(unsupported).expression,
        "Unsupported default properties enabled",
    )?;
    let mut invalid = entry;
    invalid.class = unsupported;
    check(
        data.set(invalid).is_err(),
        "Unsupported saved defaults accepted",
    )?;
    let native = root.join("default-created.drawer");
    crate::files::save(&mut blank, &native).map_err(|e| e.to_string())?;
    let reopened =
        crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == blank.snapshot(),
        "Default-created layer native reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn selected_properties_kind_isolation_new_document_reset_restart_and_no_content() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
