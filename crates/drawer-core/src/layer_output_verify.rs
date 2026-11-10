use super::*;
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
    layer_comps::{self as comps, Command, Policy},
    layers::{self, Kind, Target},
};
use std::fs;
fn capture(d: &mut Document, name: &str) -> Result<u64, String> {
    let p = comps::prepare(
        d,
        Command::Capture {
            name: name.into(),
            replace: None,
        },
    )?;
    let id = p.selected.unwrap();
    d.commit_state(p.state)?;
    Ok(id)
}
fn has(d: &Document, id: u64, key: Key, hidden: bool) -> bool {
    d.scene
        .layer_comps
        .comps
        .iter()
        .find(|c| c.id == id)
        .is_some_and(|c| c.entries.iter().any(|e| e.key == key && e.hidden == hidden))
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 3,
            height: 2,
            dpi: 299.75,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.title = "작품.drawer".into();
    d.layers[0].name = "밑그림".into();
    d.layers[0].pixels.fill([220, 30, 60, 255]);
    capture(&mut d, "표시")?;
    let state = p::prepare_patch(
        &d,
        &[p::Target::Raster(1)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    capture(&mut d, "숨김")?;
    Ok(d)
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("layer-output-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let c = AtomicBool::new(false);
        let mut d = fixture()?;
        let ids = d
            .scene
            .layer_comps
            .comps
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
        let history = d.history_counts();
        d.set_layer_comp_policy(Policy::Current)?;
        if d.history_counts() != history {
            return Err("Comp policy entered drawing Undo".into());
        }
        let before = d.snapshot();
        let created = layers::prepare(
            &d,
            layers::Command::Create {
                kind: Kind::Raster,
                name: "새층 🎨".into(),
                parent: None,
            },
        )?;
        let new = created.selected;
        d.commit_state(created.state)?;
        let key = Key::Layer(match new {
            Target::Raster(id) => p::Target::Raster(id),
            Target::Vector(id) => p::Target::Vector(id),
        });
        if !has(&d, ids[0], key, true) || !has(&d, ids[1], key, false) {
            return Err("New layer current-only entries".into());
        }
        let after = d.snapshot();
        if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
            return Err("New layer/comp entries not atomic Undo/Redo".into());
        }
        let duplicate = layers::prepare(&d, layers::Command::Duplicate(new))?;
        let duplicate_key = Key::Layer(match duplicate.selected {
            Target::Raster(id) => p::Target::Raster(id),
            Target::Vector(id) => p::Target::Vector(id),
        });
        d.commit_state(duplicate.state)?;
        if !has(&d, ids[0], duplicate_key, true) || !has(&d, ids[1], duplicate_key, false) {
            return Err("Duplicate comp policy".into());
        }
        let folder = crate::layer_folders::prepare(
            &d,
            crate::layer_folders::Command::Create {
                name: "폴더".into(),
                parent: None,
            },
        )?;
        let folder_key = Key::Layer(folder.selected);
        d.commit_state(folder.state)?;
        if !has(&d, ids[0], folder_key, true) || !has(&d, ids[1], folder_key, false) {
            return Err("Folder comp policy".into());
        }
        d.set_layer_comp_policy(Policy::All)?;
        let copied = crate::selection_transfer::copy(&d, 0, B)?;
        let pasted = crate::paste::prepare_packet(&d, &copied, copied.origin.unwrap_or([0; 2]), B)?;
        d.commit_state(pasted)?;
        let pasted_key = Key::Layer(p::raster(&d, d.layers.len() - 1));
        if !has(&d, ids[0], pasted_key, false) || !has(&d, ids[1], pasted_key, false) {
            return Err("Paste All-comp policy".into());
        }
        let storage =
            crate::storage::UserStorage::at(dir.join("user")).map_err(|e| e.to_string())?;
        Policy::Current.save(&storage)?;
        if Policy::load(&storage)? != Policy::Current {
            return Err("Comp default restart".into());
        }
        let native = dir.join("policy.drawer");
        let expected = d.snapshot();
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        if crate::files::open(&native, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            != expected
        {
            return Err("Comp native123 roundtrip".into());
        }
        let mut downgraded = fs::read(&native).map_err(|e| e.to_string())?;
        downgraded[8..12].copy_from_slice(&122u32.to_le_bytes());
        let bad = dir.join("old.drawer");
        fs::write(&bad, downgraded).map_err(|e| e.to_string())?;
        if crate::files::open(&bad, B).is_ok() {
            return Err("Older native version accepted new comp metadata".into());
        }
        let before = d.snapshot();
        let identity = (d.history_counts(), d.path.clone(), d.dirty, d.revision);
        let settings = Settings {
            pattern: "{title}-{index}-{comp}".into(),
            ..Default::default()
        };
        let plan = super::prepare(&[d.clone()], &dir, &settings, Mode::Comps, true, B, &c)?;
        if plan.files.len() != 2 || !plan.matches(&[d.clone()]) {
            return Err("Comp output source guards".into());
        }
        let names = plan
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        let report = plan.publish(&c);
        if report.files.iter().any(|f| f.result.is_err()) {
            return Err("Comp output publication".into());
        }
        let plan = super::prepare(&[d.clone()], &dir, &settings, Mode::Comps, true, B, &c)?;
        if plan.publish(&c).files.iter().any(|f| f.result.is_ok()) {
            return Err("Comp output overwrote prior outputs".into());
        }
        let plain = Settings {
            pattern: "same".into(),
            ..Default::default()
        };
        if super::prepare(&[d.clone()], &dir, &plain, Mode::Comps, true, B, &c).is_ok() {
            return Err("Duplicate names accepted".into());
        }
        let forbidden = Settings {
            pattern: "CON".into(),
            ..Default::default()
        };
        if super::prepare(&[d.clone()], &dir, &forbidden, Mode::Comps, true, B, &c).is_ok() {
            return Err("Reserved name accepted".into());
        }
        let settings = Settings {
            pattern: "layer-{index}-{layer}".into(),
            ..Default::default()
        };
        let plan = super::prepare(&[d.clone()], &dir, &settings, Mode::Layers, true, B, &c)?;
        if plan.files.len() < d.layers.len() {
            return Err("Hidden layer omitted from explicit layer output".into());
        }
        c.store(true, std::sync::atomic::Ordering::Relaxed);
        if plan.publish(&c).files.iter().any(|f| f.result.is_ok()) {
            return Err("Canceled layer output wrote files".into());
        }
        c.store(false, std::sync::atomic::Ordering::Relaxed);
        let mut missing = d.clone();
        missing.scene.layer_comps.comps[0]
            .entries
            .push(comps::Entry {
                key: Key::Layer(p::Target::Raster(99999)),
                hidden: false,
                opacity: 100,
                blend: Default::default(),
                through: None,
            });
        let s = Settings {
            pattern: "missing-{index}".into(),
            ..Default::default()
        };
        let plan = super::prepare(&[missing], &dir, &s, Mode::Comps, true, B, &c)?;
        if !plan.files[0]
            .notices
            .iter()
            .any(|s| s.contains("Missing comp layer"))
        {
            return Err("Missing comp layer not reported".into());
        }
        if d.snapshot() != before
            || identity != (d.history_counts(), d.path.clone(), d.dirty, d.revision)
            || names.iter().any(|p| !p.is_file())
        {
            return Err("Output changed live visibility/history/path".into());
        }
        Ok(())
    })();
    result.and(crate::asset_catalog::cleanup_verification(root, &dir))
}
#[cfg(test)]
mod tests {
    #[test]
    fn creation_policies_batch_names_visibility_native_and_cancel() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
