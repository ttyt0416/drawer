use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::Target,
    layer_search::Key,
    material_layer::Source,
    material_transform::{self as core, Mode, Settings},
    scene::{Affine, Rect},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn options(mode: Mode) -> Settings {
    let mut s = Settings {
        mode,
        pivot: [4.; 2],
        scale: [1.25, 0.75],
        angle: 25.,
        skew: [10., -5.],
        free: Affine([1.1, 0.2, 0.1, 0.9, 2., -1.]),
        quad: [[0.1, 0.05], [1.1, 0.1], [0.9, 1.1], [-0.05, 0.9]],
        ..Default::default()
    };
    s.mesh[4] = [0.6, 0.4];
    s
}
pub fn material() -> Result<(Document, Key), String> {
    let mut d = crate::search_verify::page()?;
    let mut layer = crate::material_mapping_verify::image();
    layer.area = Rect {
        min: [0.; 2],
        max: [8.; 2],
    };
    let (state, id) = crate::material_layer::prepare(&d, None, "Image transform", layer)?;
    state.restore(&mut d);
    Ok((d, Key::Object(id)))
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    crate::material_mapping_verify::verify(root)?;
    for mode in Mode::ALL {
        let (mut d, key) = material()?;
        let source = d.scene.objects[0].clone();
        let before = d.snapshot();
        let s = options(mode);
        let prepared = core::prepare(&d, key, &s)?;
        check(d.snapshot() == before, "Transform preparation mutated page")?;
        d.commit_state(prepared.state)?;
        let after = d.snapshot();
        check(
            matches!((&source,&d.scene.objects[0]),(crate::scene::PageObject::Material(a),crate::scene::PageObject::Material(b)) if a.source==b.source),
            "Transform changed immutable source pixels",
        )?;
        roundtrip(root, &mut d, "material-transform.drawer", &after)?;
        check(
            d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
            "Transform not one Undo/Redo",
        )?;
        masked(root, mode, true)?;
        masked(root, mode, false)?;
    }
    let fixed = Settings {
        mode: Mode::Scale,
        pivot: [3., 5.],
        scale: [2., 3.],
        ..Default::default()
    }
    .matrix()?;
    check(
        fixed.map([3., 5.]) == [3., 5.] && fixed.map([4., 6.]) == [5., 8.],
        "Scale pivot moved",
    )?;
    let free = Settings {
        mode: Mode::Free,
        pivot: [3., 5.],
        free: Affine([1., 0., 0., 1., 7., 9.]),
        ..Default::default()
    }
    .matrix()?;
    check(
        free.map([3., 5.]) == [10., 14.],
        "Free transform lost translation",
    )?;
    vector(root)?;
    locks()?;
    Ok(())
}
fn roundtrip(
    root: &std::path::Path,
    d: &mut Document,
    name: &str,
    expected: &crate::sizing::DocumentState,
) -> Result<(), String> {
    let path = root.join(name);
    crate::files::save(d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == *expected,
        "Transform native roundtrip",
    )
}
fn masked(root: &std::path::Path, mode: Mode, linked: bool) -> Result<(), String> {
    let mut d = crate::search_verify::page()?;
    let path = crate::file_objects_verify::source(
        root,
        "masked-transform-source.drawer",
        [150, 170, 190, 255],
    )?;
    let (mut state, _, _) =
        crate::file_sources::create(&d, &path, "Masked source", Default::default())?;
    let key = Key::Layer(Target::Raster(1));
    state.scene.file_objects.entries[0].target = key;
    state.restore(&mut d);
    let old_cache = d.scene.file_objects.get(key).unwrap().cache.clone();
    let w = if let Source::Image(i) = &old_cache.source {
        i.width
    } else {
        unreachable!()
    };
    let h = if let Source::Image(i) = &old_cache.source {
        i.height
    } else {
        unreachable!()
    };
    let mask = crate::layer_effects::Mask {
        space: Default::default(),
        offset: [0.; 2],
        enabled: true,
        linked,
        inverted: false,
        coverage: crate::selection::Mask {
            runs: (0..h)
                .map(|y| crate::selection::Run {
                    x: 0,
                    y: y as i32,
                    length: (w / 2).max(1),
                    alpha: if y % 2 == 0 { 128 } else { 255 },
                })
                .collect(),
        },
    };
    d.scene.layer_effects.rasters.entry(1).or_default().mask = Some(mask.clone());
    let before = d.snapshot();
    let s = options(mode);
    let prepared = core::prepare(&d, key, &s)?;
    if !linked {
        check(
            prepared.state.scene.layer_effects.rasters[&1].mask.as_ref() == Some(&mask),
            "Unlinked mask moved",
        )?;
    }
    d.commit_state(prepared.state)?;
    let after = d.snapshot();
    let next = d.scene.file_objects.get(key).unwrap();
    let next_mask = d.scene.layer_effects.rasters[&1].mask.as_ref().unwrap();
    check(
        next.path == path.canonicalize().map_err(|e| e.to_string())?
            && next.cache.source == old_cache.source,
        "Transform changed source path or RGBA",
    )?;
    if linked {
        if mode.nonlinear() {
            for y in 0..16 {
                for x in 0..16 {
                    let point = [x as f64 + 0.5, y as f64 + 0.5];
                    if let Some(local) = next.cache.local(point) {
                        let expected = mask.factor(old_cache.transform.map(local));
                        let got = next_mask.factor(point);
                        check(
                            (expected - got).abs() <= 1. / 255. + 1e-6,
                            "Linked nonlinear mask sampling differs from source field",
                        )?;
                    }
                }
            }
        } else {
            let matrix = s.matrix()?;
            for point in [[0.5, 0.5], [1.5, 1.5], [3.5, 2.5]] {
                check(
                    (next_mask.factor(matrix.map(point)) - mask.factor(point)).abs() < 1e-6,
                    "Linked affine mask did not follow pivot transform",
                )?;
            }
        }
    }
    roundtrip(root, &mut d, "masked-material-transform.drawer", &after)?;
    check(d.undo() && d.snapshot() == before, "Masked transform Undo")?;
    Ok(())
}
fn vector(root: &std::path::Path) -> Result<(), String> {
    for mode in Mode::ALL {
        for preserve in [false, true] {
            let mut d = crate::search_verify::page()?;
            let id = d.scene.lines.add_layer("Width preserved")?;
            d.scene
                .lines
                .add_path(id, vec![[2., 2.], [6., 6.]], false, 2., [200, 40, 60, 255])?;
            let key = Key::Layer(Target::Vector(id));
            let before = d.snapshot();
            let mut s = options(mode);
            s.preserve_width = preserve;
            let p = core::prepare(&d, key, &s)?;
            d.commit_state(p.state)?;
            let matrix = s.matrix()?;
            let expected = if preserve {
                2.
            } else {
                2. * (matrix.0[0] * matrix.0[3] - matrix.0[1] * matrix.0[2])
                    .abs()
                    .sqrt()
            };
            check(
                (d.scene.lines.layers[0].paths[0].width - expected).abs() < 1e-9,
                "Vector width preservation policy",
            )?;
            check(
                d.scene.lines.layers[0].paths[0].points
                    != before.scene.lines.layers[0].paths[0].points,
                "Vector mode did not transform its geometry",
            )?;
            let after = d.snapshot();
            roundtrip(root, &mut d, "vector-material-transform.drawer", &after)?;
            check(d.undo() && d.snapshot() == before, "Vector transform Undo")?;
        }
    }
    Ok(())
}
fn locks() -> Result<(), String> {
    let (mut d, key) = material()?;
    let s = options(Mode::Scale);
    let mut info = d.scene.catalog.info(key).cloned().unwrap_or_default();
    info.properties.pixels_locked = true;
    d.scene.catalog.set(key, info.clone());
    check(
        core::prepare(&d, key, &s).is_ok(),
        "Pixel lock blocked a coordinate-only material transform",
    )?;
    info.properties.position_locked = true;
    d.scene.catalog.set(key, info.clone());
    let before = d.snapshot();
    check(
        core::prepare(&d, key, &s).is_err() && d.snapshot() == before,
        "Position lock bypassed",
    )?;
    info.properties.position_locked = false;
    info.properties.locked = true;
    d.scene.catalog.set(key, info);
    check(core::prepare(&d, key, &s).is_err(), "Full lock bypassed")?;
    // A gradient ID must never resolve to an unrelated material object index.
    let unsupported = Key::Gradient(1);
    check(
        core::prepare(&d, unsupported, &s).is_err(),
        "Non-material source resolved to another object",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn all_modes_pivots_widths_linked_fixed_masks_native_locks_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
