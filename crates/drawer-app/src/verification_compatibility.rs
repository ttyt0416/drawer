use super::editor::check;
use drawer_core::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    storage::UserStorage,
};
use serde::Serialize;
use std::path::Path;
fn setting<T: Serialize + Clone>(
    root: &Path,
    name: &str,
    value: &T,
    save: impl Fn(T) -> Result<(), String>,
) -> Result<serde_json::Value, String> {
    save(value.clone())?;
    let path = root.join("settings").join(name);
    let original = std::fs::read(&path).map_err(|e| e.to_string())?;
    for future in [false, true] {
        let mut json = serde_json::to_value(value).map_err(|e| e.to_string())?;
        if future {
            json["version"] = 999.into();
        } else {
            json["futureUnknownSetting"] = serde_json::json!({"retained":"원본🙂"});
        }
        let bytes = serde_json::to_vec(&json).map_err(|e| e.to_string())?;
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        check(
            save(value.clone()).is_err()
                && std::fs::read(&path).map_err(|e| e.to_string())? == bytes,
            "Future/unknown setting was replaced",
        )?;
    }
    std::fs::write(&path, &original).map_err(|e| e.to_string())?;
    save(value.clone())?;
    Ok(
        serde_json::json!({"file":name,"futureVersionRefusedUnchanged":true,"unknownFieldRefusedUnchanged":true,"knownVersionWritable":true}),
    )
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let storage = UserStorage::at(root.join("한글 compatibility🙂")).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    macro_rules! verify {
        ($ty:path,$file:expr) => {{
            let value = <$ty>::default();
            records.push(setting(storage.root(), $file, &value, |v| {
                v.save(&storage).map_err(|e| e.to_string())
            })?);
        }};
    }
    verify!(drawer_core::workspaces::Store, "workspaces.json");
    verify!(drawer_core::ui_scale::Settings, "interface.json");
    verify!(drawer_core::aids::AidPreferences, "aids.json");
    verify!(
        drawer_core::navigator_preferences::Preferences,
        "navigator.json"
    );
    verify!(drawer_core::panels::Settings, "panels.json");
    verify!(drawer_core::interpolation::Settings, "interpolation.json");
    verify!(drawer_core::display_view::Preferences, "display.json");
    let layout = drawer_core::view_layout::Layout::empty();
    records.push(setting(storage.root(), "views.json", &layout, |v| {
        v.save(&storage).map_err(|e| e.to_string())
    })?);
    verify!(drawer_core::monitor_color::Settings, "monitor-color.json");
    verify!(drawer_core::canvas_size::PresetBook, "canvas-presets.json");
    verify!(drawer_core::resource_settings::Settings, "resources.json");
    verify!(crate::local_compute::Settings, "local-compute.json");
    let mut doc = Document::create(
        CanvasSpec {
            width: 2,
            height: 1,
            dpi: 144.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let current = root.join("current.drawer");
    drawer_core::files::save(&mut doc, &current).map_err(|e| e.to_string())?;
    let original = std::fs::read(&current).map_err(|e| e.to_string())?;
    let mut legacy = original.clone();
    let placement = 32 + 4 + doc.layers[0].name.len();
    legacy.drain(placement..placement + 8);
    drawer_core::save_generation::verify::legacy_version(&mut legacy, 1);
    legacy.truncate(32 + 4 + doc.layers[0].name.len() + 8 + 8);
    let legacy_path = root.join("legacy-v1.drawer");
    std::fs::write(&legacy_path, &legacy).map_err(|e| e.to_string())?;
    let mut old = drawer_core::files::open(&legacy_path, B).map_err(|e| e.to_string())?;
    check(
        old.snapshot() == doc.snapshot()
            && std::fs::read(&legacy_path).map_err(|e| e.to_string())? == legacy,
        "Legacy open changed appearance/source",
    )?;
    drawer_core::files::save(&mut old, &root.join("migrated-copy.drawer"))
        .map_err(|e| e.to_string())?;
    check(
        std::fs::read(&legacy_path).map_err(|e| e.to_string())? == legacy,
        "Migration overwrote the source",
    )?;
    let future = root.join("future.drawer");
    let mut bytes = original.clone();
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&future, &bytes).map_err(|e| e.to_string())?;
    check(
        drawer_core::files::open(&future, B).is_err()
            && std::fs::read(&future).map_err(|e| e.to_string())? == bytes,
        "Future Native was accepted or changed",
    )?;
    let known = drawer_core::workspaces::Store::load(&storage)
        .map_err(|e| e.to_string())?
        .ok_or("Settings restart lost workspaces")?;
    known.save(&storage).map_err(|e| e.to_string())?;
    // Reuse hash-bound fresh-process native roundtrips, including missing local fonts/resources.
    let native_root = root.join("restart");
    std::fs::create_dir(&native_root).map_err(|e| e.to_string())?;
    let native = super::native::run(&native_root)?;
    Ok(
        serde_json::json!({"settings":records,"legacyNativeVersion":1,"migrationWritesSeparateCopy":true,"futureNativeRejectedUnchanged":true,"restartNative":native}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn legacy_restart_future_settings_unknown_fields_and_original_preservation() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
