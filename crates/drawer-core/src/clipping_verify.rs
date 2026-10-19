use crate::{
    clipping as c,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::{self as p, Target},
    paper::{Folder, Node},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make(colors: &[[u8; 4]]) -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for color in colors {
        let mut l = d.layers[0].clone();
        l.pixels.fill(*color);
        d.layers.push(l);
    }
    d.layers.remove(0);
    Ok(d)
}
fn apply(
    d: &mut Document,
    targets: &[Target],
    alpha: Option<bool>,
    clip: Option<bool>,
) -> Result<(), String> {
    let s = c::prepare(d, targets, alpha, clip)?;
    d.commit_state(s)?;
    Ok(())
}
fn pixel(d: &Document) -> [u8; 4] {
    crate::render::Renderer::new(d).pixel([0.5, 0.5])
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for (base, top, expected) in [
        ([0, 0, 255, 128], [255, 0, 0, 128], [128, 0, 127, 128]),
        ([0, 0, 255, 128], [255, 0, 0, 255], [255, 0, 0, 128]),
        ([7, 8, 9, 0], [255, 0, 0, 255], [0; 4]),
        ([0, 0, 255, 255], [255, 0, 0, 0], [0, 0, 255, 255]),
    ] {
        let mut d = make(&[base, top])?;
        let before = d.snapshot();
        apply(&mut d, &[Target::Raster(2)], None, Some(true))?;
        check(
            pixel(&d) == expected && d.composite_pixel(0, 0) == expected,
            "Independent clipping premultiplied RGBA",
        )?;
        check(
            d.undo() && d.snapshot() == before && d.redo() && pixel(&d) == expected,
            "Clipping single Undo/Redo",
        )?;
    }
    let mut d = make(&[[0, 0, 255, 128], [255, 0, 0, 255], [0, 255, 0, 128]])?;
    apply(
        &mut d,
        &[Target::Raster(3), Target::Raster(2)],
        None,
        Some(true),
    )?;
    check(
        pixel(&d) == [127, 128, 0, 128]
            && d.scene.clipping.binding(Target::Raster(3)) == Some(Target::Raster(1)),
        "Chain fixed alpha and unordered batch",
    )?;
    check(
        crate::layer_pick::candidates(&d, [0.5, 0.5], Default::default()).first()
            == Some(&crate::thumbnail::Target::Raster(2)),
        "Clipped contribution picking",
    )?;
    apply(&mut d, &[Target::Raster(2)], None, Some(false))?;
    check(
        d.scene.clipping.binding(Target::Raster(3)) == Some(Target::Raster(2))
            && pixel(&d) == [127, 128, 0, 255],
        "Release middle rebase",
    )?;
    check(d.undo(), "Undo release")?;
    let before = d.snapshot();
    let s = crate::layers::prepare(
        &d,
        crate::layers::Command::Move {
            target: crate::layers::Target::Raster(1),
            parent: None,
            position: 2,
        },
    )?;
    d.commit_state(s.state)?;
    check(
        !c::active(&d, Target::Raster(2)) && pixel(&d) == [0, 0, 255, 128],
        "Moved base stays inactive",
    )?;
    check(d.undo() && d.snapshot() == before, "Move base Undo")?;
    let s = p::prepare_patch(
        &d,
        &[Target::Raster(1)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(s)?;
    check(
        pixel(&d) == [0; 4]
            && crate::layer_pick::candidates(&d, [0.5, 0.5], Default::default()).is_empty(),
        "Hidden base masks chain/picking",
    )?;
    let s = crate::layers::prepare(
        &d,
        crate::layers::Command::Delete(crate::layers::Target::Raster(1)),
    )?;
    d.commit_state(s.state)?;
    check(
        pixel(&d) == [0; 4] && !c::active(&d, Target::Raster(2)),
        "Deleted hidden base must not retarget",
    )?;
    let saved = root.join("clipping-orphan.drawer");
    crate::files::save(&mut d, &saved).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&saved, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Native orphan reopen",
    )?;
    check(
        d.undo() && d.undo() && d.snapshot() == before,
        "Deletion and hidden base Undo",
    )?;
    d.scene.paper.layers.entry(1).or_default().pattern = crate::texture::Pattern {
        source: 1,
        strength: 0.8,
        ..Default::default()
    };
    let line = d.scene.lines.add_layer("Line")?;
    d.scene.lines.add_path(
        line,
        vec![[0., 0.5], [7., 0.5]],
        false,
        1.,
        [20, 30, 40, 255],
    )?;
    let r = crate::render::Renderer::new(&d);
    let pixels = (0..8)
        .flat_map(|y| (0..8).map(move |x| (x, y)))
        .map(|(x, y)| r.pixel([x as f64 + 0.5, y as f64 + 0.5]))
        .collect::<Vec<_>>();
    let state = crate::sizing::prepare(
        &d.snapshot(),
        crate::sizing::Change::Rotate90 { clockwise: true },
        DEFAULT_DOCUMENT_BUDGET,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    d.commit_state(state)?;
    let r = crate::render::Renderer::new(&d);
    for y in 0..8 {
        for x in 0..8 {
            check(
                r.pixel([(7 - y) as f64 + 0.5, x as f64 + 0.5]) == pixels[y * 8 + x],
                "Quarter-turn texture/line clipping invariance",
            )?;
        }
    }
    let native = root.join("clipping-rotated.drawer");
    crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Native clipping/texture reopen",
    )?;
    let png = root.join("clipping.png");
    crate::png_export::export(&d, &png, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let bytes = image::open(&png)
        .map_err(|e| e.to_string())?
        .to_rgba8()
        .into_raw();
    let r = crate::render::Renderer::new(&d);
    check(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .all(|(i, p)| *p == r.pixel([(i % 8) as f64 + 0.5, (i / 8) as f64 + 0.5])),
        "Screen/PNG exact clipping",
    )?;
    let mut folder = make(&[[0, 0, 255, 255], [255, 0, 0, 255], [0, 255, 0, 255]])?;
    folder.scene.paper.folders.push(Folder {
        name: "Group".into(),
        children: vec![Node::Raster(1), Node::Raster(2)],
        through: false,
        paper: Default::default(),
    });
    folder.scene.paper.roots = vec![Node::Raster(0), Node::Folder(0)];
    check(
        c::prepare(&folder, &[Target::Raster(2)], None, Some(true)).is_err(),
        "Clipping cannot cross folder boundary",
    )?;
    apply(&mut folder, &[Target::Raster(3)], None, Some(true))?;
    let s = p::prepare_patch(
        &folder,
        &[Target::Raster(3)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    folder.commit_state(s)?;
    let original = folder.snapshot();
    let solo = c::solo(&folder, Some(0))?;
    folder.commit_state(solo)?;
    check(
        p::properties(&folder, Target::Raster(1)).hidden
            && p::properties(&folder, Target::Raster(3)).hidden
            && !p::properties(&folder, Target::Raster(2)).hidden
            && pixel(&folder) == [255, 0, 0, 255],
        "Solo preserves hidden child",
    )?;
    let s = c::solo(&folder, None)?;
    folder.commit_state(s)?;
    check(
        folder.snapshot() == original,
        "Solo restores exact saved flags",
    )?;
    folder.scene.paper.folders[0].through = true;
    check(
        c::prepare(&folder, &[Target::Folder(0)], None, Some(true)).is_err(),
        "Through folder clipping rejected",
    )?;
    let mut vector = make(&[[0; 4]])?;
    let a = vector.scene.lines.add_layer("Base")?;
    let b = vector.scene.lines.add_layer("Top")?;
    for (id, color) in [(a, [0, 0, 255, 128]), (b, [255, 0, 0, 128])] {
        vector
            .scene
            .lines
            .add_path(id, vec![[0., 0.5], [7., 0.5]], false, 1., color)?;
    }
    apply(&mut vector, &[Target::Vector(b)], None, Some(true))?;
    check(
        pixel(&vector) == [128, 0, 127, 128],
        "Vector clipping same alpha rule",
    )?;
    let mut alpha = make(&[[30, 40, 50, 128]])?;
    alpha.layers[0].pixels[0] = [7, 8, 9, 0];
    apply(&mut alpha, &[Target::Raster(1)], Some(true), None)?;
    let original = alpha.snapshot();
    check(
        !alpha.edit_pixel(0, 0, 0, [255, 0, 0, 255])
            && alpha.edit_pixel(0, 1, 0, [255, 0, 0, 255])
            && alpha.layers[0].pixels[1] == [255, 0, 0, 128],
        "Pixel alpha preservation/hidden RGB",
    )?;
    check(
        alpha.undo() && alpha.snapshot() == original,
        "Protected pixel Undo",
    )?;
    for erase in [false, true] {
        let state = crate::bucket::prepare(
            &alpha,
            0,
            None,
            [200, 20, 10, 255],
            erase,
            false,
            Default::default(),
        )?;
        check(
            state.layers[0].pixels[0] == [7, 8, 9, 0]
                && state.layers[0].pixels.iter().skip(1).all(|p| p[3] == 128),
            "Fill/eraser preserve alpha",
        )?;
        if erase {
            check(state == original, "Eraser alpha protected no-op")?;
        }
    }
    let state = crate::shapes::prepare(
        &alpha,
        crate::shapes::Target::Raster(0),
        &[[0., 0.], [4., 4.]],
        crate::shapes::Settings {
            paint: crate::shapes::Paint::CreateFill,
            fill: [200, 20, 10, 255],
            ..Default::default()
        },
    )?;
    check(
        state.layers[0]
            .pixels
            .iter()
            .zip(&original.layers[0].pixels)
            .all(|(a, b)| a[3] == b[3]),
        "Shape shares alpha protection",
    )?;
    let mut invalid = alpha.snapshot();
    invalid.layers[0].pixels[1][3] = 255;
    check(
        alpha.commit_state(invalid).is_err() && alpha.snapshot() == original,
        "State alpha bypass rejected",
    )?;
    let mut style = crate::brush_presets::Preset::capture(
        &crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        },
        crate::assets::BrushTip::PixelPen,
        "Alpha",
    )?
    .style([200, 20, 10, 255]);
    style.shape.size = 1.;
    style.pressure.size = false;
    style.pressure.opacity = false;
    let mut stroke = crate::painting::Stroke::begin(&alpha, 0, style)?;
    stroke.sample(&alpha, [1.5, 0.5])?;
    stroke.commit(&mut alpha)?;
    check(
        alpha.layers[0].pixels[1][3] == 128 && alpha.layers[0].pixels[1][0] > 30,
        "Brush alpha protection",
    )?;
    let file = root.join("alpha-protection.drawer");
    crate::files::save(&mut alpha, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == alpha.snapshot(),
        "Protected alpha native reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn clipping_alpha_folder_solo_rotation_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
