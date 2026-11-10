use super::*;
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties::Target,
    layer_search::Key,
    scene::PageObject,
};
use std::sync::atomic::AtomicUsize;
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn image(path: &Path, size: [u32; 2], color: [u8; 4]) -> Result<(), String> {
    image::save_buffer(
        path,
        &color.repeat(size[0] as usize * size[1] as usize),
        size[0],
        size[1],
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}
pub fn fixture(root: &Path) -> Result<Index, String> {
    let dir = root.join("assets");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    fs::create_dir(dir.join("sub")).map_err(|e| e.to_string())?;
    image(&dir.join("a.png"), [2, 1], [180, 20, 70, 128])?;
    fs::copy(dir.join("a.png"), dir.join("sub/a.png")).map_err(|e| e.to_string())?;
    fs::write(dir.join("broken.png"), b"broken").map_err(|e| e.to_string())?;
    fs::write(
        dir.join("colors.gpl"),
        b"GIMP Palette\nName: Local\n1 2 3 RGB\n",
    )
    .map_err(|e| e.to_string())?;
    let mut gradient = crate::gradient_ramp::Library::default();
    gradient.save(
        None,
        "Stops",
        crate::gradient::Settings {
            ramp: Some(crate::gradient_ramp::Ramp::two(
                [10, 20, 30, 255],
                [100, 150, 200, 0],
            )),
            ..Default::default()
        },
    )?;
    gradient.export(&dir.join("gradient.json"))?;
    scan(
        &dir,
        true,
        None,
        &AtomicBool::new(false),
        &AtomicUsize::new(0),
    )
}
pub fn index_files(root: &Path) -> Result<(), String> {
    let index = fixture(root)?;
    check(
        index.entries.len() == 5
            && index
                .entries
                .iter()
                .filter(|e| e.duplicate_of.is_some())
                .count()
                == 1
            && index.get(Path::new("broken.png"))?.reason.is_some(),
        "Scan count / duplicates / broken asset",
    )?;
    let mut meta = index.get(Path::new("a.png"))?.metadata.clone();
    meta.name = "한글 소재".into();
    meta.tags = vec!["paper".into(), "ink".into()];
    meta.pinned = true;
    meta.favorite = true;
    let edited = index.edited(Path::new("a.png"), meta)?;
    let rows = edited.search("한글 ink", Some(Kind::Image), "", true, true, Sort::Uses);
    check(
        rows.len() == 1 && edited.entries[rows[0]].path == Path::new("a.png"),
        "Catalog type/name/tag/favorite search",
    )?;
    let mut sub = edited.get(Path::new("sub/a.png"))?.metadata.clone();
    sub.uses = 20;
    let used = edited.edited(Path::new("sub/a.png"), sub)?;
    check(
        used.entries[used.search("", None, "", true, false, Sort::Uses)[0]].path
            == Path::new("a.png"),
        "Pin must precede usage sort",
    )?;
    check(
        used.search("", None, "sub", false, false, Sort::Path).len() == 1,
        "Logical subfolder filter",
    )?;
    let file = root.join("asset-index.json");
    let token = used.write(&file, None, false)?;
    check(Index::read(&file)?.0 == used, "Index persistence")?;
    let old = fs::read(&file).map_err(|e| e.to_string())?;
    check(
        used.write(&file, Some("stale"), false).is_err() && fs::read(&file).unwrap() == old,
        "Stale index overwrite",
    )?;
    let temp = file.with_extension("asset-index-write.tmp");
    fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
    check(
        used.write(&file, Some(&token), false).is_err()
            && fs::read(&temp).unwrap() == b"foreign"
            && fs::read(&file).unwrap() == old,
        "Foreign index temp changed",
    )?;
    fs::remove_file(temp).map_err(|e| e.to_string())?;
    let source = fs::read(used.root.join("a.png")).unwrap();
    check(
        forget(&used, Path::new("a.png"), true, false, 2).is_err()
            && used.root.join("a.png").exists(),
        "Unconfirmed source removal",
    )?;
    let (forgotten, movement) = forget(&used, Path::new("a.png"), false, true, 2)?;
    check(
        movement.is_none() && forgotten.root.join("a.png").exists(),
        "List-only deletion changed source",
    )?;
    let refreshed = scan(
        &used.root,
        true,
        Some(&forgotten),
        &AtomicBool::new(false),
        &AtomicUsize::new(0),
    )?;
    check(
        refreshed.get(Path::new("a.png")).is_err(),
        "Forgotten entry reappeared on refresh",
    )?;
    let (removed, movement) = forget(&used, Path::new("a.png"), true, true, 2)?;
    let movement = movement.ok_or("Missing reversible source move")?;
    let token = removed.write(&file, Some(&token), false)?;
    check(
        !used.root.join("a.png").exists()
            && fs::read(used.root.join(&movement.trash)).unwrap() == source,
        "Trash did not preserve source bytes",
    )?;
    let before = Arc::new(used.clone());
    let after = Arc::new(removed.clone());
    let mut history = History::default();
    history.record(before.clone(), after.clone(), Some(movement.clone()));
    fs::write(used.root.join("a.png"), b"foreign original").map_err(|e| e.to_string())?;
    check(
        history.undo(&after, &file, Some(&token)).is_err()
            && fs::read(used.root.join("a.png")).unwrap() == b"foreign original",
        "Undo replaced a foreign original",
    )?;
    fs::remove_file(used.root.join("a.png")).map_err(|e| e.to_string())?;
    let (restored, token) = history
        .undo(&after, &file, Some(&token))?
        .ok_or("Missing library Undo")?;
    check(
        restored.as_ref() == &used && fs::read(used.root.join("a.png")).unwrap() == source,
        "Library Undo/source restore",
    )?;
    let (again, _token) = history
        .redo(&restored, &file, Some(&token))?
        .ok_or("Missing library Redo")?;
    check(
        again.as_ref() == &removed && !used.root.join("a.png").exists(),
        "Library Redo/source move",
    )?;
    let record = used.root.join(&movement.receipt);
    check(
        restore_record(&used.root, &record, false).is_err(),
        "Unconfirmed trash restoration",
    )?;
    restore_record(&used.root, &record, true)?;
    check(
        fs::read(used.root.join("a.png")).unwrap() == source,
        "Persistent trash record restore",
    )?;
    let from = checked(&used.root, Path::new("sub/a.png"), true)?;
    let to = checked(&used.root, Path::new("moved.png"), false)?;
    fs::rename(from, to).map_err(|e| e.to_string())?;
    let moved = scan(
        &used.root,
        true,
        Some(&used),
        &AtomicBool::new(false),
        &AtomicUsize::new(0),
    )?;
    check(
        moved.get(Path::new("sub/a.png"))?.reason.is_some()
            && moved.get(Path::new("moved.png"))?.available(),
        "Moved source availability",
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let locked = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(used.root.join("a.png"))
            .map_err(|e| e.to_string())?;
        let denied = scan(
            &used.root,
            true,
            Some(&used),
            &AtomicBool::new(false),
            &AtomicUsize::new(0),
        )?;
        check(
            denied.get(Path::new("a.png"))?.reason.is_some(),
            "Access-denied/shared-locked asset was applicable",
        )?;
        drop(locked);
    }
    check(
        checked(&used.root, Path::new("../escape.png"), false).is_err(),
        "Catalog path traversal",
    )?;
    Ok(())
}
pub fn transfers(root: &Path) -> Result<(), String> {
    let index = fixture(root)?;
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 600.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let original = d.snapshot();
    let path = Path::new("a.png");
    let cancel = AtomicBool::new(false);
    let first = prepare(
        &index,
        path,
        &d,
        None,
        ReplaceSize::KeepPixels,
        B / 2,
        &cancel,
    )?;
    let first_id = first.object;
    check(d.snapshot() == original, "Preview changed original")?;
    d.commit_state(first.state)?;
    let second = prepare(
        &index,
        path,
        &d,
        None,
        ReplaceSize::KeepPixels,
        B / 2,
        &cancel,
    )?;
    let second_id = second.object;
    d.commit_state(second.state)?;
    let saved = d.snapshot();
    let before_count = d.history_counts();
    let renderer = crate::render::Renderer::new(&d);
    let rendered = (0..8)
        .flat_map(|y| (0..8).map(move |x| [x, y]))
        .map(|[x, y]| renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]))
        .collect::<Vec<_>>();
    drop(renderer);
    image(&index.root.join(path), [4, 2], [20, 180, 40, 255])?;
    check(
        prepare(
            &index,
            path,
            &d,
            None,
            ReplaceSize::KeepPixels,
            B / 2,
            &cancel,
        )
        .is_err()
            && d.snapshot() == saved,
        "Changed asset was applied without refresh",
    )?;
    let refreshed = scan(
        &index.root,
        true,
        Some(&index),
        &cancel,
        &AtomicUsize::new(0),
    )?;
    let selected = d
        .scene
        .catalog
        .objects
        .index(first_id, d.scene.objects.len())
        .unwrap();
    let other = d
        .scene
        .catalog
        .objects
        .index(second_id, d.scene.objects.len())
        .unwrap();
    let PageObject::Material(old) = &d.scene.objects[selected] else {
        return Err("Missing image object".into());
    };
    let transform = old.transform;
    let area = old.area;
    let other_before = d.scene.objects[other].clone();
    let replacement = prepare(
        &refreshed,
        path,
        &d,
        Some(first_id),
        ReplaceSize::FitExisting,
        B / 2,
        &cancel,
    )?;
    check(d.snapshot() == saved, "Replacement preview changed page")?;
    fs::remove_file(index.root.join(path)).map_err(|e| e.to_string())?;
    replacement.check(&d)?;
    d.commit_state(replacement.state)?;
    let PageObject::Material(layer) = &d.scene.objects[selected] else {
        return Err("Replacement lost image object".into());
    };
    check(
        layer.transform == transform
            && layer.area == area
            && d.scene.objects[other] == other_before
            && d.history_counts().0 == before_count.0 + 1,
        "Replacement transform/scope/one Undo",
    )?;
    let after = d.snapshot();
    check(
        d.undo() && d.snapshot() == saved && d.redo() && d.snapshot() == after,
        "Replacement Undo/Redo",
    )?;
    check(
        prepare(
            &refreshed,
            path,
            &d,
            None,
            ReplaceSize::KeepPixels,
            B / 2,
            &cancel,
        )
        .is_err(),
        "Missing asset applied",
    )?;
    let file = root.join("snapshot.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let transported = root.join("other-computer");
    fs::create_dir(&transported).map_err(|e| e.to_string())?;
    let copy = transported.join("art.drawer");
    fs::copy(&file, &copy).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&copy, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == after
            && reopened.scene.catalog.object_info[&first_id]
                .asset_version
                .as_ref()
                .is_some_and(Version::valid),
        "Independent native asset snapshot after source removal",
    )?;
    let mut old = fs::read(&file).unwrap();
    crate::save_generation::verify::legacy_version(&mut old, 121);
    let old_path = root.join("false-121.drawer");
    fs::write(&old_path, old).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old_path, B).is_err(),
        "Asset references accepted as older native format",
    )?;
    let copied = register(
        &index.root,
        &d,
        Key::Layer(Target::Raster(d.scene.layer_ids.id(0))),
        "Current selection",
        &cancel,
    )?;
    check(
        index.root.join(&copied).is_file() && d.snapshot() == after,
        "Register mutated current raster",
    )?;
    let copied_object = register(
        &index.root,
        &d,
        Key::Object(first_id),
        "Image layer snapshot",
        &cancel,
    )?;
    check(
        index.root.join(copied_object).is_file() && d.snapshot() == after,
        "Register nonraster object mutated original",
    )?;
    check(
        prepare(
            &index,
            Path::new("colors.gpl"),
            &d,
            None,
            ReplaceSize::KeepPixels,
            B / 2,
            &cancel,
        )
        .is_err()
            && register(
                &index.root,
                &d,
                Key::Layer(Target::Raster(d.scene.layer_ids.id(0))),
                "cancel",
                &AtomicBool::new(true),
            )
            .is_err(),
        "Wrong type or cancel accepted",
    )?;

    let precise_path = root.join("precise.png");
    let high = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(
        1,
        1,
        vec![1234, 34567, 65000, 32768],
    )
    .ok_or("Precise fixture")?;
    high.save(&precise_path).map_err(|e| e.to_string())?;
    let precise = crate::image_import::ImageSource::read(&precise_path, B / 4, &cancel)
        .map_err(|e| e.to_string())?
        .decode(0, true, B / 4, &cancel)
        .map_err(|e| e.to_string())?
        .document(B / 4)
        .map_err(|e| e.to_string())?;
    let registered = register(
        &index.root,
        &precise,
        Key::Layer(Target::Raster(precise.scene.layer_ids.id(0))),
        "Precise snapshot",
        &cancel,
    )?;
    let loaded =
        crate::image_import::ImageSource::read(&index.root.join(registered), B / 4, &cancel)
            .map_err(|e| e.to_string())?
            .decode(0, true, B / 4, &cancel)
            .map_err(|e| e.to_string())?;
    check(
        loaded.bits == 16
            && loaded
                .color
                .as_ref()
                .is_some_and(|c| c.pixels[0] == [1234, 34567, 65000, 32768]),
        "Registration discarded precise channel values",
    )?;
    // Library refresh did not repaint the second already-embedded image.
    let restored = crate::layer_merge::preview(&saved, d.current_frame)?;
    let renderer = crate::render::Renderer::new(&restored);
    check(
        (0..8)
            .flat_map(|y| (0..8).map(move |x| [x, y]))
            .map(|[x, y]| renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]))
            .collect::<Vec<_>>()
            == rendered,
        "Library change repainted existing pixels",
    )?;
    Ok(())
}
pub fn materials(root: &Path) -> Result<(), String> {
    use crate::{
        assets::BrushTip,
        material_library::{FIRST_ID, Kind as MaskKind, Library},
        mouse_pressure::{Replay, Sample},
        painting::Style,
    };
    let library = Library::default()
        .register_mask("Tip", MaskKind::Tip, 2, 1, vec![255, 40])
        .map_err(|e| e.to_string())?
        .register_mask("Texture", MaskKind::Texture, 2, 1, vec![20, 240])
        .map_err(|e| e.to_string())?;
    let style = Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: crate::tip_sequence::Settings {
            tips: [FIRST_ID, 0, 0, 0, 0, 0, 0, 0],
            enabled: true,
            ..Default::default()
        },
        filters: Default::default(),
        profile: Default::default(),
        texture: crate::texture::Pattern {
            source: FIRST_ID + 1,
            ..Default::default()
        },
        material: Default::default(),
        opacity: 1.,
        hardness: 1.,
        spacing: 0.15,
        shape: crate::brush_cursor::Shape {
            size: 4.,
            ..Default::default()
        },
        tip: BrushTip::Pen,
        color: [100, 50, 200, 255],
        smoothing: 0.,
        pressure: Default::default(),
    };
    let mut d = Document::create(
        CanvasSpec {
            width: 12,
            height: 12,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let mut replay = Replay::begin(&d, 0, style, Default::default())?
        .with_materials(Arc::new(library.clone()))?;
    replay.push(Sample {
        point: [3., 3.],
        micros: Some(0),
    })?;
    replay.push(Sample {
        point: [8., 7.],
        micros: Some(10000),
    })?;
    let metadata = replay.metadata();
    check(
        metadata.material_versions.len() == 2,
        "Tip/texture content versions not captured",
    )?;
    replay
        .render(&d, true)?
        .commit_mouse(&mut d, metadata.clone())?;
    let before = d.snapshot();
    let pixels = d.layers[0].pixels.clone();
    let mut changed = serde_json::to_value(&library).map_err(|e| e.to_string())?;
    changed["assets"][0]["values"] = serde_json::json!([10, 255]);
    let changed: Library = serde_json::from_value(changed).map_err(|e| e.to_string())?;
    check(
        changed.valid() && material_versions(&style, &changed) != metadata.material_versions,
        "Material modification did not change its content version",
    )?;
    let _removed = changed.remove(FIRST_ID).map_err(|e| e.to_string())?;
    check(
        d.snapshot() == before && d.layers[0].pixels == pixels,
        "Library change/delete repainted prior brush pixels",
    )?;
    let mut scene = d.scene.clone();
    let mut newer = metadata;
    newer.material_versions = material_versions(&style, &changed);
    scene.append_mouse(newer)?;
    check(
        scene.mouse_strokes.len() == 2,
        "Different material versions were coalesced",
    )?;
    let path = root.join("brush-versions.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == before,
        "Brush version references lost in native",
    )?;
    Ok(())
}
pub fn thousands(root: &Path) -> Result<(), String> {
    let dir = root.join("many");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    image(&dir.join("source.png"), [1, 1], [10, 20, 30, 255])?;
    for n in 0..2000 {
        fs::copy(dir.join("source.png"), dir.join(format!("{n:04}.png")))
            .map_err(|e| e.to_string())?;
    }
    let progress = AtomicUsize::new(0);
    let index = scan(&dir, false, None, &AtomicBool::new(false), &progress)?;
    check(
        index.entries.len() == 2001
            && index
                .entries
                .iter()
                .filter(|e| e.duplicate_of.is_some())
                .count()
                == 2000,
        "Thousands-file scan/duplicate count",
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AtomicUsize::new(0));
    let (c, p, path) = (cancel.clone(), progress.clone(), dir.clone());
    let worker = std::thread::spawn(move || scan(&path, false, None, &c, &p));
    for _ in 0..2000 {
        if progress.load(Ordering::Relaxed) > 8 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    cancel.store(true, Ordering::Relaxed);
    check(
        worker.join().map_err(|_| "Scan worker panic")?.is_err() && index.entries.len() == 2001,
        "Mid-scan cancel published a partial index",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("asset-catalog-core");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        for (name, run) in [
            ("index", index_files as fn(&Path) -> Result<(), String>),
            ("transfers", transfers),
            ("materials", materials),
            ("many", thousands),
            ("presets", preset_stability),
        ] {
            let part = dir.join(name);
            fs::create_dir(&part).map_err(|e| e.to_string())?;
            run(&part)?;
        }
        Ok(())
    })();
    result.and(cleanup_verification(root, &dir))
}
#[cfg(test)]
mod tests {
    #[test]
    fn folder_index_metadata_duplicates_missing_permissions_store_and_reversible_delete() {
        let t = crate::test_support::TempDir::new();
        super::index_files(&t.0).unwrap();
    }
    #[test]
    fn embedded_versions_selected_replace_cancel_undo_and_native_transport() {
        let t = crate::test_support::TempDir::new();
        super::transfers(&t.0).unwrap();
    }
    #[test]
    fn brush_tip_texture_hashes_settings_snapshots_and_library_independence() {
        let t = crate::test_support::TempDir::new();
        super::materials(&t.0).unwrap();
    }
    #[test]
    fn two_thousand_files_and_mid_scan_cancel_do_not_publish_partial_index() {
        let t = crate::test_support::TempDir::new();
        super::thousands(&t.0).unwrap();
    }
}

pub fn preset_stability(root: &Path) -> Result<(), String> {
    let mut vector = crate::vector_tip_verify::fixture()?;
    let layer = vector.scene.lines.layers[0].id;
    let other = vector.scene.lines.layers[0].paths[1].clone();
    let selected = crate::vector_edit::Selection {
        paths: vec![vector.scene.lines.layers[0].paths[0].id],
        ..Default::default()
    };
    let original = vector.snapshot();
    vector.commit_state(crate::vector_tip::prepare(
        &vector, layer, &selected, "Diamond",
    )?)?;
    check(
        vector.scene.catalog.vectors[&layer]
            .asset_version
            .as_ref()
            .is_some_and(Version::valid)
            && vector.scene.lines.layers[0].paths[1] == other,
        "Vector tip version or selected scope",
    )?;
    let after = vector.snapshot();
    check(
        vector.undo()
            && vector.snapshot() == original
            && vector.redo()
            && vector.snapshot() == after,
        "Vector version one Undo",
    )?;
    let mut presets = crate::text_format::Presets::default();
    presets.save("Local mixed".into(), Default::default(), false)?;
    let mut settings = crate::text_layout_verify::settings();
    settings.style.rich = Some(Box::new(crate::text_format::Rich {
        spans: vec![],
        mapping: Some(presets.values[0].1.clone()),
    }));
    let mut text = crate::text_layout_verify::fixture()?;
    let prepared = crate::text_layout::prepare(&text, None, &settings)?;
    let id = prepared.id;
    text.commit_state(prepared.state)?;
    let saved = text.snapshot();
    check(
        text.scene.catalog.object_info[&id]
            .asset_version
            .as_ref()
            .is_some_and(Version::valid),
        "Mixed-font mapping version missing",
    )?;
    let render = |d: &Document| {
        let r = crate::render::Renderer::new(d);
        (0..d.height)
            .flat_map(|y| (0..d.width).map(move |x| [x, y]))
            .map(|[x, y]| r.pixel([x as f64 + 0.5, y as f64 + 0.5]))
            .collect::<Vec<_>>()
    };
    let before = render(&text);
    presets.values[0].1.base = crate::text_layout::Font::Hack;
    presets.values[0].1.rules[0].ratio = 1.7;
    presets.values.clear();
    check(
        text.snapshot() == saved && render(&text) == before,
        "Mixed-font preset change/delete repainted text",
    )?;
    let file = root.join("mixed-font-snapshot.drawer");
    crate::files::save(&mut text, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == saved && render(&reopened) == before,
        "Mixed-font snapshot/native render changed",
    )?;
    Ok(())
}
#[cfg(test)]
mod preset_tests {
    #[test]
    fn vector_tip_and_mixed_font_values_hashes_survive_local_preset_changes() {
        let t = crate::test_support::TempDir::new();
        super::preset_stability(&t.0).unwrap();
    }
}
