use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    page_parts as parts,
    render::Renderer,
    scene::{PageObject, Rect},
};
use std::{path::Path, sync::Arc};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn fixture(index: u32) -> Document {
    let mut d = Document::create_with_color(
        CanvasSpec {
            width: 32,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
        Default::default(),
    )
    .unwrap();
    let pixel =
        crate::color_precision::Sample::from16([12345 + index as u16 * 1000, 23456, 34567, 65535]);
    d.layers[0].precise.as_mut().unwrap().fill(pixel.rgba16());
    d.layers[0].pixels.fill(pixel.rgba8());
    d.scene.objects.push(PageObject::Text {
        layout: None,
        baseline: [1., 20.],
        text: format!("Page {index}"),
        size_px: 9.,
        color: [0, 0, 0, 255],
    });
    d.scene.lines.next_id = 3;
    d.scene.lines.layers.push(crate::line_tools::Layer {
        fill: Default::default(),
        id: 1,
        name: "crossing vector".into(),
        hidden: false,
        locked: false,
        paths: vec![crate::line_tools::Path {
            svg: None,
            tip: Default::default(),
            pressure: Default::default(),
            fill: None,
            stroke: true,
            definition: None,
            id: 2,
            points: vec![[2., 10.], [29., 55.]],
            closed: false,
            width: 2.,
            color: [240, 0, 30, 255],
        }],
    });
    d.scene.camera = Some(crate::scene::Camera {
        page_bounds: Rect {
            min: [0.; 2],
            max: [32., 64.],
        },
        position: [0.; 3],
        pixels_per_unit: 1.,
    });
    d.scene.worlds.push(crate::scene::WorldMesh {
        vertices: vec![[-10., -30., 0.], [10., 30., 0.]],
        edges: vec![[0, 1]],
        color: [40, 200, 80, 255],
    });
    d.animation = Some(crate::animation::Timeline {
        fps: crate::animation::Fps {
            numerator: if index == 0 { 24 } else { 30000 },
            denominator: if index == 0 { 1 } else { 1001 },
        },
        frames: 3,
        tracks: vec![crate::animation::Track {
            name: "independent timing".into(),
            cells: vec![crate::animation::Cell {
                name: "cell".into(),
                width: 2,
                height: 64,
                offset: [15, 0],
                pixels: Arc::new(vec![[0, 0, 255, 160]; 128]),
            }],
            exposures: vec![
                crate::animation::Exposure { frame: 0, cell: 0 },
                crate::animation::Exposure { frame: 1, cell: 1 },
            ],
            end: 3,
        }],
        audio: vec![],
    });
    d.animation.as_mut().unwrap().tracks[0]
        .cells
        .push(crate::animation::Cell {
            name: "second held cel".into(),
            width: 2,
            height: 64,
            offset: [15, 0],
            pixels: Arc::new(vec![[0, 220, 80, 160]; 128]),
        });
    d.current_frame = 1;
    d
}
pub fn verify(root: &Path) -> Result<(), String> {
    let mut draft = fixture(0);
    draft.scene.catalog.rasters.entry(1).or_default().draft = true;
    let plain = fixture(1);
    let combined = parts::merge(&[(1, &draft), (2, &plain)], false, B)?;
    let original_export = Renderer::for_export(&draft)?;
    let output = Renderer::for_export(&combined)?;
    for y in 0..64 {
        for x in 0..32 {
            check(
                output.pixel([x as f64 + 0.5, y as f64 + 0.5])
                    == original_export.pixel([x as f64 + 0.5, y as f64 + 0.5]),
                "Contained export included draft source pixels",
            )?;
        }
    }
    drop(output);
    drop(original_export);
    let a = fixture(0);
    let b = fixture(1);
    let before = [a.snapshot(), b.snapshot()];
    let mut merged = parts::merge(&[(10, &a), (20, &b)], true, B)?;
    check(
        merged.width == 64
            && merged.height == 64
            && merged.animation.is_none()
            && merged.scene.camera.is_none()
            && merged.scene.page_parts.entries.len() == 2,
        "Merged page dimensions or outer camera/timeline",
    )?;
    for (i, p) in merged.scene.page_parts.entries.iter().enumerate() {
        let child = p.open()?;
        check(
            child.snapshot() == before[i]
                && child.current_frame == 1
                && child.scene.worlds.len() == 1
                && child.scene.lines.layers.len() == 1,
            "Typed contents, source camera/timeline or current frame lost",
        )?;
    }
    let expected = [Renderer::new(&a), Renderer::new(&b)];
    let result = Renderer::new(&merged);
    check(result.error().is_none(), "Combined renderer failed")?;
    for y in 0..64 {
        for x in 0..64 {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            check(
                result.pixel(p) == expected[(x / 32) as usize].pixel([p[0] % 32., p[1]]),
                "Merged pixels changed source text/vector/world/timeline appearance",
            )?;
        }
    }
    drop(result);
    let split = parts::split(&merged, 23, B)?;
    for (i, d) in split.iter().enumerate() {
        let r = Renderer::new(d);
        let m = Renderer::new(&merged);
        for y in 0..d.height {
            for x in 0..d.width {
                check(
                    r.pixel([x as f64 + 0.5, y as f64 + 0.5])
                        == m.pixel([
                            x as f64 + 0.5 + if i == 0 { 0. } else { 23. },
                            y as f64 + 0.5,
                        ]),
                    "Split changed retained crossing objects",
                )?;
            }
        }
    }
    let copied = crate::layer_move::prepare(
        &merged,
        &[crate::layer_search::Key::Object(
            merged.scene.page_parts.entries[0].object,
        )],
        [1, 0],
        true,
    )?;
    let crate::layer_search::Key::Object(new_id) = copied.selected else {
        return Err("Contained copy target type".into());
    };
    check(
        copied.state.scene.page_parts.get(new_id).is_some()
            && copied.state.scene.page_parts.entries.len() == 3,
        "Generic layer duplication lost contained source",
    )?;
    let working = crate::layer_merge::preview(&copied.state, 0)?;
    let raster = crate::raster_conversion::prepare(
        &working,
        crate::layer_search::Key::Object(new_id),
        crate::raster_conversion::Action::Rasterize(Default::default()),
    )?;
    check(
        raster.state.scene.page_parts.get(new_id).is_none() && raster.state.scene.valid(),
        "Raster conversion left an orphan contained source",
    )?;
    let mut half = fixture(1);
    half.dpi = 600.;
    let mixed = parts::merge(&[(1, &a), (2, &half)], true, B)?;
    check(
        mixed.width == 48 && mixed.scene.page_parts.entries[1].document().unwrap().dpi == 600.,
        "Physical-DPI page placement changed source DPI",
    )?;
    let mut nested = parts::merge(&[(1, &a), (2, &b)], false, B)?;
    for n in 1..4 {
        nested = parts::merge(&[(n, &nested), (20, &b)], false, B)?;
    }
    check(
        nested.scene.page_parts.depth() == 4
            && parts::merge(&[(1, &nested), (2, &b)], false, B).is_err(),
        "Nested page depth boundary ignored",
    )?;
    let old = merged.snapshot();
    let id = merged.scene.page_parts.entries[0].object;
    let mut edit = merged.scene.page_parts.entries[0].open()?;
    edit.edit_pixel(0, 0, 0, [255, 0, 0, 255]);
    let edit_before = edit.snapshot();
    merged.commit_state(parts::replace(&merged, id, &edit, B)?)?;
    check(
        merged.scene.page_parts.entries[0].open()?.snapshot() == edit_before
            && merged.scene.page_parts.entries[1].open()?.snapshot() == before[1]
            && split[0].scene.page_parts.entries[0].open()?.snapshot() == before[0],
        "Contained edit propagated to sibling/split copy",
    )?;
    merged.undo();
    check(merged.snapshot() == old, "Contained edit was not one Undo")?;
    let dir = root.join("contained-pages");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let path = dir.join("combined.drawer");
        crate::files::save(&mut merged, &path).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&path).unwrap();
        check(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) == 94,
            "Contained page native version",
        )?;
        let opened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == merged.snapshot(),
            "Contained native payload roundtrip",
        )?;
        let mut old = bytes.clone();
        old[8..12].copy_from_slice(&93u32.to_le_bytes());
        let path = dir.join("old-header.drawer");
        std::fs::write(&path, old).unwrap();
        check(
            crate::files::open(&path, B).is_err(),
            "Old header accepted contained pages",
        )?;
        let mut bad = bytes;
        let last = bad.len() - 1;
        bad[last] ^= 1;
        std::fs::write(&path, bad).unwrap();
        check(
            crate::files::open(&path, B).is_err(),
            "Corrupt contained payload accepted",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)?;
    check(
        a.snapshot() == before[0] && b.snapshot() == before[1],
        "Page operation changed originals",
    )?;
    check(
        parts::split(&merged, 0, B).is_err()
            && parts::split(&merged, merged.width, B).is_err()
            && parts::merge(&[], true, B).is_err()
            && parts::merge(&[(1, &a), (2, &b)], true, 8).is_err(),
        "Empty/edge/limit page operation accepted",
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn editable_containers_preserve_precise_crossing_text_vector_world_camera_independent_fps_and_native()
     {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
