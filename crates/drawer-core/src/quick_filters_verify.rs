use crate::{
    batch_filters::Task,
    document::DEFAULT_DOCUMENT_BUDGET,
    layer_properties::Target,
    quick_filters::{self as quick, Input, Kind, Mode, Output, Recipe, Route},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<crate::document::Document, String> {
    let mut d = crate::spatial_blur_verify::fixture()?;
    let mut created = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Other layer".into(),
            parent: None,
        },
    )?;
    created.state.layers[1].placement = None;
    created.state.layers[1].pixels = vec![[0; 4]; 17 * 17];
    d.commit_state(created.state)?;
    Ok(d)
}
pub fn identity(route: Route) -> Recipe {
    Recipe {
        route,
        task: Task::Detail(crate::detail_filter::Settings {
            operation: crate::detail_filter::Operation::Sharpen { strength: 0. },
            selection: false,
            options: Default::default(),
        }),
    }
}
pub fn routes() -> Result<(), String> {
    let mut d = fixture()?;
    d.layers[1].pixels[8 * 17 + 8] = [0, 0, 255, 64];
    let original = d.snapshot();
    let target = Target::Raster(d.scene.layer_ids.id(0));
    let current = identity(Route::default());
    let copied = quick::prepare(&d, target, current, DEFAULT_DOCUMENT_BUDGET, |_, _| Ok(()))?;
    check(
        d.snapshot() == original
            && copied.state.layers.len() == 3
            && copied.state.layers[..2] == original.layers
            && copied.state.layers[2].pixels == original.layers[0].pixels,
        "Current input did not preserve source/native result",
    )?;
    let composite = identity(Route {
        input: Input::Composite,
        kind: Kind::Composite,
        ..Default::default()
    });
    let result = quick::prepare(
        &d,
        target,
        composite,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    let expected = crate::render::Renderer::new(&d).sample([8.5; 2])?.rgba8();
    check(
        result.state.layers[2].pixels[144] == expected
            && result.state.layers[2].pixels[144] != copied.state.layers[2].pixels[144],
        "Composite input did not include other layer",
    )?;
    let zero = quick::prepare(
        &d,
        target,
        identity(Route {
            mode: Mode::Pixels,
            output: Output::Current,
            ..Default::default()
        }),
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        zero.state == original,
        "Current pixel strength0 changed history state",
    )?;
    check(
        quick::prepare(&d, target, current, DEFAULT_DOCUMENT_BUDGET, |_, _| {
            Err("cancel".into())
        })
        .is_err()
            && d.snapshot() == original,
        "Route cancellation changed source",
    )?;
    check(
        !Route {
            mode: Mode::Quick,
            output: Output::Current,
            ..Default::default()
        }
        .valid()
            && !Route {
                input: Input::Composite,
                output: Output::Current,
                kind: Kind::Composite,
                ..Default::default()
            }
            .valid(),
        "Invalid quick/composite current output accepted",
    )?;
    let count = d.history_counts();
    d.commit_state(copied.state)?;
    check(
        d.history_counts().0 == count.0 + 1,
        "New result not one Undo",
    )?;
    d.undo();
    check(d.snapshot() == original, "New result Undo")?;
    let objects = crate::raster_conversion_verify::mixed()?;
    let snapshot = objects.snapshot();
    let object = Target::Object(1);
    let new = quick::prepare(
        &objects,
        object,
        identity(Route {
            kind: Kind::Object,
            ..Default::default()
        }),
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        !new.loss
            && new.state.scene.objects == snapshot.scene.objects
            && new.state.layers[..snapshot.layers.len()] == snapshot.layers,
        "Object new result lost editable source",
    )?;
    let rasterized = quick::prepare(
        &objects,
        object,
        identity(Route {
            mode: Mode::Pixels,
            output: Output::Current,
            kind: Kind::Object,
            ..Default::default()
        }),
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        rasterized.loss
            && rasterized.state.scene.objects.len() < snapshot.scene.objects.len()
            && objects.snapshot() == snapshot,
        "Current object route missing raster loss plan",
    )?;
    Ok(())
}
pub fn live_and_delete() -> Result<(), String> {
    let mut d = fixture()?;
    let stack = crate::surface_effects::Stack {
        effects: vec![crate::surface_effects::Entry {
            operation: crate::surface_effects::Operation::defaults()[1],
            enabled: true,
            scope: None,
        }],
        ..Default::default()
    };
    d.commit_state(crate::surface_effects::prepare(
        &d,
        0,
        stack,
        |_, _| Ok(()),
    )?)?;
    let original = d.snapshot();
    let source = Target::Raster(d.scene.layer_ids.id(0));
    let p = quick::prepare(
        &d,
        source,
        identity(Route::default()),
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    d.commit_state(p.state)?;
    let fixed = d.layers[2].clone();
    let before = crate::render::Renderer::new(&d).sample([5.5, 8.5])?;
    let mut edited = d.snapshot();
    edited.layers[0].pixels.fill([0; 4]);
    d.commit_state(edited)?;
    let after = crate::render::Renderer::new(&d).sample([5.5, 8.5])?;
    check(
        before != after && d.layers[2] == fixed && d.scene.surface_effects.rasters.len() == 1,
        "Live source recalculation or fixed result independence",
    )?;
    let restored = crate::layer_merge::preview(&original, 0)?;
    let target = crate::layers::Target::Raster(restored.scene.layer_ids.id(0));
    check(
        quick::attached_effects(&restored, target),
        "Missing source effect dependency",
    )?;
    let drop = quick::delete_source(&restored, target, false)?;
    let keep = quick::delete_source(&restored, target, true)?;
    check(
        restored.snapshot() == original
            && drop.state.layers.len() == 1
            && keep.state.layers.len() == 2
            && drop.state.scene.surface_effects.rasters.is_empty()
            && keep.state.scene.surface_effects.rasters.is_empty(),
        "Delete/retain raster plans or preview isolation",
    )?;
    check(
        keep.state
            .layers
            .iter()
            .any(|l| l.pixels.iter().any(|p| p[3] > 0)),
        "Retained raster lost page appearance",
    )?;
    let mut edited = crate::layer_merge::preview(&original, 0)?;
    let count = edited.history_counts();
    edited.commit_state(keep.state)?;
    check(
        edited.history_counts().0 == count.0 + 1,
        "Source deletion not one Undo",
    )?;
    edited.undo();
    check(edited.snapshot() == original, "Source deletion Undo")?;
    check(
        serde_json::from_str::<crate::surface_effects::Stack>(r#"{"effects":[],"source_id":1}"#)
            .is_err(),
        "Unsupported/cyclic live source reference accepted",
    )?;
    let mut invalid = original.scene;
    invalid.effect_algorithm = quick::EffectAlgorithm(2);
    check(!invalid.valid(), "Unknown effect algorithm accepted")?;
    Ok(())
}
pub fn disk(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    d.color = crate::color_storage::Config {
        depth: crate::color_precision::Depth::Sixteen,
        ..Default::default()
    };
    for layer in &mut d.layers {
        let mut high: Vec<_> = layer
            .pixels
            .iter()
            .map(|p| crate::color_precision::Sample::from8(*p).rgba16())
            .collect();
        high[144] = [12345, 23456, 34567, 1];
        layer.pixels = high
            .iter()
            .map(|p| crate::color_precision::Sample::from16(*p).rgba8())
            .collect();
        layer.precise = Some(high);
    }
    let source = Target::Raster(d.scene.layer_ids.id(0));
    let recipe = identity(Route::default());
    let prepared = quick::prepare(&d, source, recipe, DEFAULT_DOCUMENT_BUDGET, |_, _| Ok(()))?;
    check(
        prepared.state.layers[2].precise.as_ref().unwrap()[144] == [12345, 23456, 34567, 1],
        "New result lost native16 alpha1 precision",
    )?;
    d.commit_state(prepared.state)?;
    let file = root.join("quick-route.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot(),
        "Quick native/cache reconstruction",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&84u32.to_le_bytes());
    let fake = root.join("quick-fake84.drawer");
    std::fs::write(&fake, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&fake, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted quick route",
    )?;
    let storage =
        crate::storage::UserStorage::at(root.join("quick-presets")).map_err(|e| e.to_string())?;
    let mut store = crate::workspaces::Store::default();
    store
        .current
        .quick_presets
        .values
        .push(("Native copy".into(), recipe));
    store.save(&storage).map_err(|e| e.to_string())?;
    let restored = crate::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Missing quick presets")?;
    check(
        restored.current.quick_presets == store.current.quick_presets,
        "Route preset restart",
    )?;
    let mut another = fixture()?;
    another.layers[0].pixels[144] = [12, 34, 56, 255];
    let result = quick::prepare(
        &another,
        Target::Raster(another.scene.layer_ids.id(0)),
        restored.current.quick_presets.values[0].1,
        DEFAULT_DOCUMENT_BUDGET,
        |_, _| Ok(()),
    )?;
    check(
        result.state.layers[2].pixels[144] == [12, 34, 56, 255],
        "Preset retained another document's source",
    )?;
    let json = serde_json::to_value(recipe).map_err(|e| e.to_string())?;
    check(
        json.get("source").is_none()
            && json.get("result").is_none()
            && json.get("target_id").is_none(),
        "Preset leaked document identities",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    routes()?;
    live_and_delete()?;
    disk(root)
}
#[cfg(test)]
mod tests {
    #[test]
    fn input_output_routes_sources_objects_loss_and_undo() {
        super::routes().unwrap();
    }
    #[test]
    fn live_recalculation_fixed_result_delete_retain_cycle_version() {
        super::live_and_delete().unwrap();
    }
    #[test]
    fn native16_presets_other_document_and_reopen() {
        let root = crate::test_support::TempDir::new();
        super::disk(&root.0).unwrap();
    }
}
