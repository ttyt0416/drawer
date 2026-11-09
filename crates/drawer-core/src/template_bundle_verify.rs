use super::*;
fn check(ok: bool, note: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(note.into()) }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("template-bundle-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let mut d = Document::create(
        crate::document::CanvasSpec {
            width: 3,
            height: 2,
            dpi: 299.75,
            background: crate::document::Background::Transparent,
        },
        1024,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].name = "한글 레이어 🎨".into();
    d.layers[0].pixels[0] = [10, 50, 90, 128];
    d.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 3,
        tracks: vec![],
        audio: vec![],
    });
    d.current_frame = 2;
    d.scene
        .objects
        .push(crate::scene::PageObject::Material(Box::new(
            crate::material_layer::Layer {
                source: crate::material_layer::Source::Image(crate::material_layer::Image {
                    width: 2,
                    height: 1,
                    pixels: std::sync::Arc::new(vec![[200, 10, 20, 128], [20, 100, 30, 255]]),
                }),
                area: crate::scene::Rect {
                    min: [0., 0.],
                    max: [2., 1.],
                },
                transform: Default::default(),
                repeat: [false; 2],
                mapping: Default::default(),
            },
        )));
    let animation = d.animation.as_mut().unwrap();
    animation.tracks.push(crate::animation::Track {
        name: "한글 셀".into(),
        cells: vec![crate::animation::Cell {
            name: "cel".into(),
            width: 1,
            height: 1,
            offset: [1, 1],
            pixels: std::sync::Arc::new(vec![[1, 2, 3, 200]]),
        }],
        exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
        end: 3,
    });
    animation.audio.push(crate::animation::Audio {
        name: "own PCM".into(),
        start: 0,
        rate: 8000,
        channels: 1,
        sample_count: 8,
        samples: std::sync::Arc::new(vec![0, 1, 2, 3, 4, 3, 2, 1]),
    });
    let original = d.snapshot();
    let identity = (d.id, d.path.clone(), d.dirty, d.history_counts());
    let masks = Library::default()
        .register_mask(
            "직접 만든 팁",
            crate::material_library::Kind::Tip,
            2,
            2,
            vec![255, 128, 64, 1],
        )
        .map_err(|e| e.to_string())?;
    for (n, kind) in Kind::ALL.into_iter().enumerate() {
        let folder = dir.join(format!("template{n}"));
        export(&d, &folder, "직접 만든 템플릿", kind, &masks, &cancel)?;
        let copy = import(&folder, crate::document::DEFAULT_DOCUMENT_BUDGET, &cancel)?;
        check(
            copy.document.snapshot() == original
                && copy.document.id != d.id
                && copy.document.path.is_none()
                && copy.document.dirty
                && copy.document.current_frame == 2
                && copy.document.history_counts() == (0, 0),
            "Template editable content/identity/history changed",
        )?;
        check(copy.masks == masks, "Archived mask dependencies changed")?;
        let merged = copy.copied_masks(&masks)?;
        check(
            merged.assets().len() == 2 && merged.assets()[0].id != merged.assets()[1].id,
            "Mask ID collision",
        )?;
        check(
            export(&d, &folder, "replacement", kind, &masks, &cancel).is_err(),
            "Existing bundle overwritten",
        )?;
    }
    let folder = dir.join("template0");
    let manifest_path = folder.join("manifest.json");
    let original_manifest = fs::read(&manifest_path).map_err(|e| e.to_string())?;
    let manifest: Manifest =
        serde_json::from_slice(&original_manifest).map_err(|e| e.to_string())?;
    for change in 0..6 {
        let mut m = manifest.clone();
        match change {
            0 => m.version = 999,
            1 => m.native_version = 999,
            2 => m.files[0].path = "../escape.drawer".into(),
            3 => m.files[1].path = "template.drawer".into(),
            4 => m.native_version = crate::files::VERSION - 1,
            _ => m.files[0].sha256 = "0".repeat(64),
        }
        fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).map_err(|e| e.to_string())?;
        check(
            import(&folder, crate::document::DEFAULT_DOCUMENT_BUDGET, &cancel).is_err(),
            "Malformed manifest accepted",
        )?;
    }
    fs::write(&manifest_path, &original_manifest).map_err(|e| e.to_string())?;
    let mask_path = folder.join("masks.json");
    let mask_bytes = fs::read(&mask_path).map_err(|e| e.to_string())?;
    fs::remove_file(&mask_path).map_err(|e| e.to_string())?;
    check(
        import(&folder, crate::document::DEFAULT_DOCUMENT_BUDGET, &cancel).is_err(),
        "Missing masks accepted",
    )?;
    let mut value: serde_json::Value = serde_json::from_slice(&mask_bytes).unwrap();
    let a = value["assets"][0].clone();
    value["assets"].as_array_mut().unwrap().push(a);
    let corrupt = serde_json::to_vec(&value).unwrap();
    fs::write(&mask_path, &corrupt).map_err(|e| e.to_string())?;
    let mut m = manifest.clone();
    m.files[1].bytes = corrupt.len();
    m.files[1].sha256 = assets::hash(&corrupt);
    fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).map_err(|e| e.to_string())?;
    check(
        import(&folder, crate::document::DEFAULT_DOCUMENT_BUDGET, &cancel).is_err(),
        "Duplicate material IDs accepted",
    )?;
    fs::write(&mask_path, mask_bytes).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, original_manifest).map_err(|e| e.to_string())?;
    cancel.store(true, Ordering::Relaxed);
    check(
        import(&folder, crate::document::DEFAULT_DOCUMENT_BUDGET, &cancel).is_err(),
        "Canceled import accepted",
    )?;
    let canceled = dir.join("canceled");
    check(
        export(&d, &canceled, "Cancel", Kind::Layers, &masks, &cancel).is_err()
            && !canceled.exists(),
        "Canceled export published",
    )?;
    check(
        d.snapshot() == original && identity == (d.id, d.path.clone(), d.dirty, d.history_counts()),
        "Template changed source document",
    )?;
    assets::cleanup_verification(root, &dir)
}
#[cfg(test)]
mod tests {
    #[test]
    fn portable_templates_validate_resources_versions_ids_cancel_and_source_integrity() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
