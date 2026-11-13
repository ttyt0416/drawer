use super::*;
fn check(ok: bool, note: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(note.into()) }
}
pub fn run(root: &Path) -> Result<(), String> {
    portable_resources(root)?;
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
pub fn portable_resources(root: &Path) -> Result<(), String> {
    use crate::{document::DEFAULT_DOCUMENT_BUDGET as B, text_layout as text};
    let dir = root.join("portable-resource-check");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let catalog = crate::font_library_verify::fixture(&dir)?;
    let entry = catalog
        .entries
        .iter()
        .find(|e| {
            matches!(e.font, text::Font::Local(_)) && e.name.to_lowercase().contains("ubuntu")
        })
        .ok_or("Portable font fixture")?;
    let font = entry.font;
    let fontpath = entry.path.clone().ok_or("Font path")?;
    let mut d = crate::text_layout_verify::fixture()?;
    let mut settings = crate::text_layout_verify::settings();
    settings.style.font = font;
    settings.style.ruby.push(text::Ruby {
        start: 0,
        end: "한글".len(),
        text: "hangul".into(),
        font,
        ..Default::default()
    });
    settings.style.rich = Some(Box::new(crate::text_format::Rich {
        mapping: Some(crate::text_format::Mapping {
            base: font,
            rules: vec![crate::text_format::Rule {
                group: crate::text_format::Group::Korean,
                from: 0,
                through: 0,
                font: text::Font::Korean,
                ratio: 1.,
            }],
        }),
        ..Default::default()
    }));
    d.commit_state(text::prepare(&d, None, &settings)?.state)?;
    let source = crate::file_sources_verify::source(&dir)?;
    let (state, _) = crate::file_objects::create(&d, &source, "Linked owned page")?;
    d.commit_state(state)?;
    let model = crate::model_formats_verify::fixture(&dir.join("model-source"), "vrm")?;
    crate::model_library::prepare(
        &d,
        crate::model_library::Edit::Import {
            path: model.clone(),
            placement: crate::model_library::Placement {
                fit: None,
                ..Default::default()
            },
        },
        B,
        &cancel,
    )?
    .commit(&mut d)?;
    let original = d.snapshot();
    let history = d.history_counts();
    let pixels = |d: &Document| -> Vec<_> {
        (0..d.height)
            .flat_map(|y| (0..d.width).map(move |x| d.composite_pixel(x, y)))
            .collect()
    };
    let before = pixels(&d);
    let keep = dir.join("keep-links");
    export(
        &d,
        &keep,
        "Keep original links",
        Kind::Layers,
        &Library::default(),
        &cancel,
    )?;
    let bundle = dir.join("portable");
    export_resources(
        &d,
        &bundle,
        "휴대 문서",
        Kind::Layers,
        &Library::default(),
        &cancel,
        true,
    )?;
    check(
        d.snapshot() == original && d.history_counts() == history,
        "Portable export changed original links/content/history",
    )?;
    let moved = dir.join("다른 사용자 폴더 🎨");
    fs::rename(&bundle, &moved).map_err(|e| e.to_string())?;
    fs::remove_file(&fontpath).map_err(|e| e.to_string())?;
    fs::remove_file(&source).map_err(|e| e.to_string())?;
    fs::remove_file(&model).map_err(|e| e.to_string())?;
    crate::text_layout::invalidate_font_data();
    let fresh =
        crate::storage::UserStorage::at(dir.join("empty-profile")).map_err(|e| e.to_string())?;
    check(
        !fresh.root().join("fonts").exists(),
        "Fresh profile unexpectedly has fonts",
    )?;
    let copy = import(&moved, B, &cancel)?;
    let mut expected = original.clone();
    for (n, link) in expected.scene.file_objects.entries.iter_mut().enumerate() {
        link.path = crate::file_browser::folder(&moved)?.join(format!("linked-{n:03}.png"));
        link.selection = Default::default();
    }
    check(
        copy.document.snapshot() == expected && pixels(&copy.document) == before,
        "Moved bundle lost text/mapping/cache/model/appearance",
    )?;
    check(
        copy.notices
            .iter()
            .any(|n| n.contains("Missing font:") && n.contains(&font.label())),
        "Missing local font lacks precise replacement guidance",
    )?;
    let kept = import(&keep, B, &cancel)?;
    check(
        kept.document.snapshot() == original && pixels(&kept.document) == before,
        "Keeping missing source links lost native caches",
    )?;
    check(
        kept.notices
            .iter()
            .any(|n| n.contains("Original source links retained")),
        "Kept source policy not explained",
    )?;
    check(
        !fs::read_dir(&moved).map_err(|e| e.to_string())?.any(|p| {
            p.is_ok_and(|p| {
                p.path()
                    .extension()
                    .is_some_and(|e| e == "ttf" || e == "otf")
            })
        }),
        "Font file redistributed without permission",
    )?;
    let manifest_path = moved.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).map_err(|e| e.to_string())?;
    let mut manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    check(
        manifest.version == 2
            && manifest.files.len() == 3
            && manifest.embedded_links.len() == 1
            && manifest
                .files
                .iter()
                .all(|f| !f.path.contains([':', '/', '\\'])),
        "Portable resources are not bounded relative names",
    )?;
    let image_path = moved.join("linked-000.png");
    let image_bytes = fs::read(&image_path).map_err(|e| e.to_string())?;
    let bad = b"owned changed PNG";
    fs::write(&image_path, bad).map_err(|e| e.to_string())?;
    manifest.files[2].bytes = bad.len();
    manifest.files[2].sha256 = assets::hash(bad);
    fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    check(
        import(&moved, B, &cancel).is_err(),
        "Rehashed PNG differs from Native cache but was accepted",
    )?;
    fs::write(&manifest_path, manifest_bytes).map_err(|e| e.to_string())?;
    fs::write(&image_path, image_bytes).map_err(|e| e.to_string())?;
    let mut reopened = copy.document;
    let stable = reopened.snapshot();
    let p = dir.join("portable-reopened.drawer");
    crate::files::save(&mut reopened, &p).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&p, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == stable,
        "Portable save/reopen changed content",
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
