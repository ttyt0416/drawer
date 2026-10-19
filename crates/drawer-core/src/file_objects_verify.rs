use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    file_objects as core,
    layer_properties::{self as p, Target},
    layer_search::Key,
    render::Renderer,
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn source(
    root: &std::path::Path,
    name: &str,
    color: [u8; 4],
) -> Result<std::path::PathBuf, String> {
    let image = crate::references::LocalImage {
        width: 2,
        height: 1,
        pixels: vec![color, [0; 4]],
    };
    let mut d = image
        .document(DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?;
    let path = root.join(name);
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    Ok(path)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let path = source(root, "file-source.drawer", [210, 40, 20, 128])?;
    let mut d = crate::layer_roles_verify::page()?;
    let before = d.snapshot();
    let (s, id) = core::create(&d, &path, "Local source")?;
    let key = Key::Object(id);
    check(d.snapshot() == before, "File preview changed source")?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([0.5; 2]) == [210, 40, 20, 128]
            && Renderer::new(&d).pixel([2.5, 0.5]) == [0; 4],
        "Linked small transparent image",
    )?;
    source(root, "file-source.drawer", [10, 30, 90, 255])?;
    check(
        Renderer::new(&d).pixel([0.5; 2]) == [210, 40, 20, 128],
        "File updated without explicit refresh",
    )?;
    let counts = d.history_counts();
    let s = core::refresh(&d, key, None)?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([0.5; 2]) == [10, 30, 90, 255]
            && d.history_counts().0 == counts.0 + 1,
        "Explicit refresh/one Undo",
    )?;
    check(
        d.undo() && Renderer::new(&d).pixel([0.5; 2]) == [210, 40, 20, 128] && d.redo(),
        "Refresh Undo/Redo",
    )?;
    let live = d.snapshot();
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    check(
        core::refresh(&d, key, None).is_err() && d.snapshot() == live,
        "Missing file changed cache",
    )?;
    check(
        core::source_folder(&path).is_ok(),
        "Missing file prevented opening its existing source folder",
    )?;
    let native = root.join("file-cache.drawer");
    crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
    let reopened =
        crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot()
            && Renderer::new(&reopened).pixel([0.5; 2]) == [10, 30, 90, 255],
        "Missing source/native embedded cache",
    )?;
    let other = source(root, "relinked.drawer", [20, 80, 40, 128])?;
    let transform = d.scene.file_objects.get(key).unwrap().cache.transform;
    let s = core::refresh(&d, key, Some(&other))?;
    d.commit_state(s)?;
    check(
        d.scene.file_objects.get(key).unwrap().cache.transform == transform
            && Renderer::new(&d).pixel([0.5; 2]) == [20, 80, 40, 128],
        "Relink lost transform/cache",
    )?;
    let s = crate::search_move::prepare(&d, key, [2, 3])?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([2.5, 3.5]) == [20, 80, 40, 128]
            && Renderer::new(&d).pixel([0.5; 2]) == [0; 4],
        "File layer move",
    )?;
    let live = d.snapshot();
    let counts = d.history_counts();
    for path in [
        std::path::Path::new("https://example.invalid/image.png"),
        std::path::Path::new(r"\\server\share\image.png"),
        std::path::Path::new("relative.png"),
        root,
    ] {
        check(
            core::refresh(&d, key, Some(path)).is_err()
                && d.snapshot() == live
                && d.history_counts() == counts,
            "Invalid local source changed cache",
        )?;
    }
    std::fs::write(root.join("bad-file.drawer"), b"not a document").map_err(|e| e.to_string())?;
    check(
        core::refresh(&d, key, Some(&root.join("bad-file.drawer"))).is_err()
            && d.snapshot() == live,
        "Failed read changed cache",
    )?;
    let mut cyclic = crate::layer_merge::preview(&d.snapshot(), 0)?;
    let cycle = root.join("cycle.drawer");
    cyclic.scene.file_objects.entries[0].path = cycle.clone();
    crate::files::save(&mut cyclic, &cycle).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&cycle, DEFAULT_DOCUMENT_BUDGET).is_ok()
            && core::read(&cycle, None).is_err()
            && core::refresh(&d, key, Some(&native)).is_err(),
        "Cycle/own-document reference accepted or cached open followed links",
    )?;
    let nested = root.join("nested.drawer");
    let mut n = crate::layer_roles_verify::page()?;
    let (s, _) = core::create(&n, &other, "Nested")?;
    n.commit_state(s)?;
    crate::files::save(&mut n, &nested).map_err(|e| e.to_string())?;
    check(
        core::read(&nested, None).is_ok(),
        "Valid nested cached source rejected",
    )?;
    raster_conversion(root)?;
    other_sources(root)?;
    gradients_worlds_and_animation(root)?;
    clipboard(root)?;
    Ok(())
}
fn gradients_worlds_and_animation(root: &std::path::Path) -> Result<(), String> {
    let mut g = crate::layer_roles_verify::page()?;
    let state = crate::gradient::prepare(
        &g,
        0,
        crate::gradient::Settings {
            end: [8., 0.],
            ..Default::default()
        },
        true,
        Default::default(),
    )?;
    g.commit_state(state)?;
    let id = g.scene.gradients.layers[0].id;
    let mut c = core::convert(&g, Key::Gradient(id), &root.join("gradient-file.drawer"))?;
    c.save(false)?;
    g.commit_state(c.state)?;
    let state = core::refresh(&g, Key::Gradient(id), None)?;
    g.commit_state(state)?;
    let mut w = crate::layer_roles_verify::page()?;
    w.scene.camera = Some(crate::scene::Camera {
        page_bounds: crate::scene::Rect {
            min: [0.; 2],
            max: [8.; 2],
        },
        position: [0.; 3],
        pixels_per_unit: 1.,
    });
    w.scene.worlds.push(crate::scene::WorldMesh {
        vertices: vec![[-2., 0., 0.], [2., 0., 0.]],
        edges: vec![[0, 1]],
        color: [20, 60, 100, 128],
    });
    let mut c = core::convert(&w, Key::World(1), &root.join("world-file.drawer"))?;
    c.save(false)?;
    w.commit_state(c.state)?;
    for (name, target) in [
        ("cell-file.drawer", Target::Cell { track: 0, cell: 0 }),
        ("track-file.drawer", Target::Track(0)),
    ] {
        let mut d = crate::layer_roles_verify::page()?;
        d.animation = Some(crate::animation::Timeline {
            fps: Default::default(),
            frames: 1,
            audio: vec![],
            tracks: vec![crate::animation::Track {
                name: "Track".into(),
                cells: vec![crate::animation::Cell {
                    name: "Frame".into(),
                    width: 2,
                    height: 1,
                    offset: [2, 3],
                    pixels: std::sync::Arc::new(vec![[120, 60, 30, 128], [0; 4]]),
                }],
                exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
                end: 1,
            }],
        });
        d.scene.layer_properties.tracks.insert(
            0,
            p::Properties {
                opacity: 60,
                ..Default::default()
            },
        );
        d.scene.layer_properties.cells.insert(
            0,
            p::Properties {
                opacity: 50,
                ..Default::default()
            },
        );
        let key = crate::layer_catalog::key(&d, target);
        let before = Renderer::new(&d).pixel([2.5, 3.5]);
        let mut c = core::convert(&d, key, &root.join(name))?;
        c.save(false)?;
        d.commit_state(c.state)?;
        let state = core::refresh(&d, key, None)?;
        d.commit_state(state)?;
        check(
            Renderer::new(&d).pixel([2.5, 3.5]) == before,
            "Track/cell cache applied opacity twice",
        )?;
        let native = root.join(format!("cached-{name}"));
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        let reopened =
            crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == d.snapshot(),
            "Animation plus file-cache native payload order",
        )?;
    }
    Ok(())
}
fn raster_conversion(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    d.layers[0].pixels[3 * 8 + 3] = [200, 80, 40, 128];
    let t = Target::Raster(1);
    d.scene.layer_properties.rasters.insert(
        1,
        p::Properties {
            opacity: 60,
            ..Default::default()
        },
    );
    d.scene.appearance.rasters.insert(
        1,
        crate::appearance::Style {
            enabled: true,
            border: 1,
            border_color: [20, 30, 40, 128],
            ..Default::default()
        },
    );
    let before = d.snapshot();
    let counts = d.history_counts();
    let path = root.join("converted-raster.drawer");
    let mut c = core::convert(&d, Key::Layer(t), &path)?;
    check(
        !path.exists() && d.snapshot() == before,
        "Conversion preflight wrote a source or changed layer",
    )?;
    c.save(false)?;
    d.commit_state(c.state)?;
    check(
        path.is_file() && d.history_counts().0 == counts.0 + 1 && p::paintable(&d, 0).is_err(),
        "File conversion save/one Undo/read-only source",
    )?;
    check(
        d.undo() && d.snapshot() == before && path.is_file() && d.redo(),
        "Conversion Undo deleted outside source or lost original",
    )?;
    let state = core::refresh(&d, Key::Layer(t), None)?;
    let preview = crate::layer_merge::preview(&state, 0)?;
    for y in 0..8 {
        for x in 0..8 {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            check(
                Renderer::new(&d).pixel(p) == Renderer::new(&preview).pixel(p),
                "Native source refresh doubled opacity/mask/display recipe",
            )?;
        }
    }
    source(root, "converted-raster.drawer", [90, 30, 10, 128])?;
    let updated = core::refresh(&d, Key::Layer(t), None)?;
    d.commit_state(updated)?;
    let rendered = d.scene.paper.render_node(
        &d,
        crate::paper::Node::Raster(0),
        DEFAULT_DOCUMENT_BUDGET / 8,
    )?;
    check(
        rendered.extent(8, 8).width == 2
            && rendered.extent(8, 8).height == 1
            && rendered.pixel(8, 8, 2, 2) == [90, 30, 10, 77],
        "Rendered-layer export used stale source pixels or old file bounds",
    )?;
    let flat = crate::layer_merge::prepare(
        &d,
        crate::layer_merge::Operation::Flatten,
        Default::default(),
    )?;
    let flat = crate::layer_merge::preview(&flat.state, 0)?;
    check(
        flat.scene.file_objects.is_default()
            && Renderer::new(&flat).pixel([2.5; 2]) == Renderer::new(&d).pixel([2.5; 2]),
        "Flatten ignored file cache or retained a consumed link",
    )?;
    let before_bake = d.snapshot();
    check(
        crate::paper::bake(&mut d, crate::paper::Node::Raster(0))?
            && d.scene.file_objects.is_default()
            && Renderer::new(&d).pixel([2.5; 2]) == [90, 30, 10, 77]
            && d.undo()
            && d.snapshot() == before_bake,
        "Bake file cache/Undo",
    )?;
    let moved = crate::search_move::prepare(&d, Key::Layer(t), [1, 2])?;
    d.commit_state(moved)?;
    check(
        Renderer::new(&d).pixel([3.5, 4.5])[3] > 0,
        "Raster file movement lost cache",
    )?;
    let dup = crate::layers::prepare(
        &d,
        crate::layers::Command::Duplicate(crate::layers::Target::Raster(1)),
    )?;
    let crate::layers::Target::Raster(new) = dup.selected else {
        return Err("Raster copy target".into());
    };
    check(
        dup.state
            .scene
            .file_objects
            .get(Key::Layer(Target::Raster(new)))
            .is_some(),
        "Raster duplicate lost file link",
    )?;
    let restored = core::restore_original(&d, Key::Layer(t))?;
    d.commit_state(restored)?;
    check(
        d.scene.file_objects.entries.is_empty(),
        "Restore original did not remove link",
    )?;
    let snapshot = d.snapshot();
    let mut collision = core::convert(&d, Key::Layer(t), &path)?;
    check(
        collision.save(false).is_err() && d.snapshot() == snapshot,
        "Existing source silently overwritten",
    )?;
    let bad = root.join("directory.drawer");
    std::fs::create_dir(&bad).map_err(|e| e.to_string())?;
    let mut failed = core::convert(&d, Key::Layer(t), &bad)?;
    check(
        failed.save(true).is_err()
            && d.snapshot() == snapshot
            && !root.join("directory.drawer.drawer-write.tmp").exists(),
        "Failed outside save changed source or leaked temporary file",
    )?;
    Ok(())
}
fn other_sources(root: &std::path::Path) -> Result<(), String> {
    let mut v = crate::line_tools_verify::fixture()?;
    let key = Key::Layer(Target::Vector(v.scene.lines.layers[0].id));
    let mut c = core::convert(&v, key, &root.join("converted-vector.drawer"))?;
    c.save(false)?;
    v.commit_state(c.state)?;
    let s = core::refresh(&v, key, None)?;
    v.commit_state(s)?;
    check(
        v.scene.file_objects.get(key).is_some() && !v.scene.lines.layers[0].paths.is_empty(),
        "Vector file source lost original paths",
    )?;
    let mut d = crate::layer_roles_verify::page()?;
    d.scene.objects.push(crate::scene::PageObject::Line {
        start: [1., 2.],
        end: [6., 2.],
        width_px: 1.,
        color: [60, 90, 120, 128],
    });
    let mut c = core::convert(&d, Key::Object(1), &root.join("converted-object.drawer"))?;
    c.save(false)?;
    d.commit_state(c.state)?;
    let mut f = crate::layer_roles_verify::page()?;
    f.layers[0].pixels[9] = [40, 80, 120, 128];
    let p = crate::layer_folders::prepare(
        &f,
        crate::layer_folders::Command::Wrap {
            name: "Group".into(),
            targets: vec![Target::Raster(1)],
        },
    )?;
    f.commit_state(p.state)?;
    let mut c = core::convert(
        &f,
        Key::Layer(p.selected),
        &root.join("converted-folder.drawer"),
    )?;
    c.save(false)?;
    f.commit_state(c.state)?;
    check(
        Renderer::new(&f).pixel([1.5; 2]) == [40, 80, 120, 128],
        "Folder file cache changed composition",
    )?;
    Ok(())
}
fn clipboard(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    let before = d.snapshot();
    let packet = crate::paste::ClipboardImage {
        image: crate::references::LocalImage {
            width: 2,
            height: 1,
            pixels: vec![[40, 80, 120, 128], [0; 4]],
        },
        origin: Some([3, 4]),
    };
    let path = root.join("clipboard-file.drawer");
    let mut c = core::from_clipboard(&d, packet, &path)?;
    c.save(false)?;
    d.commit_state(c.state)?;
    check(
        Renderer::new(&d).pixel([3.5, 4.5]) == [40, 80, 120, 128]
            && d.undo()
            && d.snapshot() == before
            && path.is_file(),
        "Clipboard position, alpha, Undo or outside file preservation",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_links_refresh_relink_missing_cycles_conversion_undo_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
