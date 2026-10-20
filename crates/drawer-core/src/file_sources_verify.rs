use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    file_sources::{self as core, Content, Selection},
    layer_properties::Target,
    layer_search::Key,
    material_layer::Source,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn source(root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([200, 20, 40, 255]);
    d.layers.push(d.layers[0].clone());
    d.layers[1].pixels.fill([20, 40, 200, 255]);
    d.scene.layer_ids.initialize(2)?;
    d.scene.paper.layers.entry(1).or_default().hidden = true;
    crate::layer_comps::prepare(
        &d,
        crate::layer_comps::Command::Capture {
            name: "Lower only".into(),
            replace: None,
        },
    )?
    .state
    .restore(&mut d);
    d.scene.paper.layers.get_mut(&1).unwrap().hidden = false;
    d.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 2,
        audio: vec![],
        tracks: vec![crate::animation::Track {
            name: "Timeline".into(),
            end: 2,
            cells: [[20, 200, 40, 255], [200, 180, 20, 255]]
                .into_iter()
                .enumerate()
                .map(|(n, p)| crate::animation::Cell {
                    name: format!("Cell {n}"),
                    width: 1,
                    height: 1,
                    offset: [1, 1],
                    pixels: std::sync::Arc::new(vec![p]),
                })
                .collect(),
            exposures: vec![
                crate::animation::Exposure { frame: 0, cell: 0 },
                crate::animation::Exposure { frame: 1, cell: 1 },
            ],
        }],
    });
    let path = root.join("source-selection.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    Ok(path)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let path = source(root)?;
    let encoded = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut newer =
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    newer.layers[0].pixels.fill([9, 8, 7, 255]);
    crate::files::save(&mut newer, &path).map_err(|e| e.to_string())?;
    let frozen = crate::files::read_native_snapshot(&path, &encoded, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    check(
        frozen.layers[0].pixels[0] == [200, 20, 40, 255],
        "Native snapshot reopened a newer path instead of captured bytes",
    )?;
    std::fs::write(&path, encoded).map_err(|e| e.to_string())?;
    let metadata = core::inspect(&path)?;
    check(
        metadata.native && metadata.frames == 2 && metadata.timeline && metadata.comps.len() == 1,
        "Source inventory",
    )?;
    for (selection, index, expected) in [
        (Selection::default(), 0, [20, 40, 200, 255]),
        (
            Selection {
                content: Content::Layer(Key::Layer(Target::Raster(1))),
                ..Default::default()
            },
            0,
            [200, 20, 40, 255],
        ),
        (
            Selection {
                comp: Some(1),
                ..Default::default()
            },
            0,
            [200, 20, 40, 255],
        ),
        (
            Selection {
                content: Content::Timeline,
                frame: 1,
                ..Default::default()
            },
            3,
            [200, 180, 20, 255],
        ),
        (
            Selection {
                content: Content::Timeline,
                frame: 1,
                ..Default::default()
            },
            0,
            [0; 4],
        ),
        (
            Selection {
                content: Content::Layer(Key::Track(1)),
                frame: 0,
                ..Default::default()
            },
            3,
            [20, 200, 40, 255],
        ),
    ] {
        let s = core::read(&path, None, &selection)?;
        check(
            s.image.pixels[index] == expected,
            "Source layer/comp/timeline/frame selected wrong pixels",
        )?;
    }
    for selection in [
        Selection {
            frame: 2,
            ..Default::default()
        },
        Selection {
            comp: Some(999),
            ..Default::default()
        },
        Selection {
            content: Content::Layer(Key::Layer(Target::Raster(999))),
            ..Default::default()
        },
    ] {
        check(
            core::read(&path, None, &selection).is_err(),
            "Missing source choice silently fell back",
        )?;
    }
    check(
        core::read(&path, Some(&path), &Default::default()).is_err(),
        "Self-reference accepted",
    )?;
    let mut d = crate::search_verify::page()?;
    let selection = Selection {
        content: Content::Layer(Key::Layer(Target::Raster(1))),
        frame: 1,
        comp: Some(1),
    };
    let (state, id, _) = core::create(&d, &path, "Selected source", selection.clone())?;
    d.commit_state(state)?;
    let missing =
        crate::file_objects_verify::source(root, "source-missing.drawer", [40, 80, 120, 255])?;
    let (state, other, _) = core::create(&d, &missing, "Keep failed cache", Default::default())?;
    d.commit_state(state)?;
    std::fs::remove_file(&missing).map_err(|e| e.to_string())?;
    let mut source =
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    source.layers[0].pixels.fill([70, 90, 110, 255]);
    crate::files::save(&mut source, &path).map_err(|e| e.to_string())?;
    let before = d.snapshot();
    let counts = d.history_counts();
    let prepared = core::refresh_all(&d)?;
    check(
        prepared.succeeded == vec![Key::Object(id)]
            && prepared.failed.len() == 1
            && prepared.failed[0].0 == Key::Object(other),
        "Partial refresh results",
    )?;
    check(
        d.snapshot() == before && d.history_counts() == counts,
        "Batch refresh preparation mutated page",
    )?;
    check(
        prepared.state.scene.file_objects.get(Key::Object(other))
            == d.scene.file_objects.get(Key::Object(other)),
        "Failed source lost cache",
    )?;
    let next = prepared
        .state
        .scene
        .file_objects
        .get(Key::Object(id))
        .unwrap();
    check(
        next.selection == selection
            && matches!(&next.cache.source,Source::Image(i)if i.pixels[0]==[70,90,110,255]),
        "Refreshed source selection/pixels",
    )?;
    prepared.validate(&d)?;
    d.commit_state(prepared.state)?;
    let after = d.snapshot();
    check(
        d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
        "Partial refresh not single Undo",
    )?;
    let file = root.join("source-selection-owner.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == after,
        "Source selection/cache native roundtrip",
    )?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    let mut old = bytes;
    old[8..12].copy_from_slice(&62u32.to_le_bytes());
    std::fs::write(&file, old).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Source selection accepted under v62 header",
    )?;
    let prepared = core::refresh_all(&d)?;
    source.layers[0].pixels.fill([30, 50, 70, 255]);
    crate::files::save(&mut source, &path).map_err(|e| e.to_string())?;
    check(
        prepared.validate(&d).is_err() && d.snapshot() == after,
        "Source modified after review was accepted",
    )?;
    // Cycles are checked through every nested native source, even when a
    // selected layer would happen not to paint that dependency at this frame.
    let a = crate::file_objects_verify::source(root, "cycle-a.drawer", [1, 2, 3, 255])?;
    let b = crate::file_objects_verify::source(root, "cycle-b.drawer", [4, 5, 6, 255])?;
    let mut first = crate::files::open(&a, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let (state, _, _) = core::create(&first, &b, "B", Default::default())?;
    first.commit_state(state)?;
    crate::files::save(&mut first, &a).map_err(|e| e.to_string())?;
    let mut second = crate::files::open(&b, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let mut link = first.scene.file_objects.entries[0].clone();
    link.path = a.clone();
    second.scene = first.scene.clone();
    second.scene.file_objects.entries = vec![link];
    crate::files::save(&mut second, &b).map_err(|e| e.to_string())?;
    check(
        core::read(&a, None, &Default::default()).is_err(),
        "Nested cycle accepted",
    )?;
    exchange(root)?;
    Ok(())
}
fn exchange(root: &std::path::Path) -> Result<(), String> {
    let path = root.join("source-pages.tiff");
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut encoder = tiff::encoder::TiffEncoder::new(file).map_err(|e| e.to_string())?;
    for p in [[210u8, 30, 50], [30, 210, 50]] {
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(1, 1, &p)
            .map_err(|e| e.to_string())?;
    }
    drop(encoder);
    let info = core::inspect(&path)?;
    check(!info.native && info.frames == 2, "Exchange page inventory")?;
    let selected = core::read(
        &path,
        None,
        &Selection {
            frame: 1,
            ..Default::default()
        },
    )?;
    check(
        selected.image.pixels[0] == [30, 210, 50, 255],
        "Exchange TIFF page selection",
    )?;
    check(
        core::read(
            &path,
            None,
            &Selection {
                comp: Some(1),
                ..Default::default()
            },
        )
        .is_err(),
        "Exchange source invented comp",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_choices_partial_refresh_native_undo_stamps_cycles_tiff() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
