use crate::{
    affine_transform as core,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    interpolation::Filter,
    layer_search::Key,
    scene::Rect,
    vector_scope::Transform,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
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
    d.layers[0].pixels[9] = [220, 20, 30, 255];
    d.layers[0].pixels[10] = [20, 210, 30, 128];
    d.layers[0].pixels[45] = [20, 30, 200, 255];
    d.scene.selection = Some(Rect {
        min: [1., 1.],
        max: [3., 2.],
    });
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let key = Key::Layer(crate::layer_properties::raster(&d, 0));
    let original = d.snapshot();
    let counts = d.history_counts();
    let t = Transform {
        pivot: [1., 1.],
        scale: [-1., 2.],
        translation: [4., 1.],
        ..Default::default()
    };
    let p = core::prepare(&d, key, true, t, false, Filter::Nearest)?;
    let r = &p.state.layers[0];
    for y in [2, 3] {
        check(
            r.pixel(8, 8, 3, y) == [20, 210, 30, 128] && r.pixel(8, 8, 4, y) == [220, 20, 30, 255],
            "Selected negative/nonuniform pivot transform differs from coordinate oracle",
        )?;
    }
    check(
        r.pixel(8, 8, 1, 1)[3] == 0
            && r.pixel(8, 8, 2, 1)[3] == 0
            && r.pixel(8, 8, 5, 5) == [20, 30, 200, 255]
            && p.state.scene.selection_coverage([3.5, 2.5]) == 255
            && p.state.scene.selection_coverage([1.5, 1.5]) == 0,
        "Cut/selection follow/outside preservation",
    )?;
    check(
        d.snapshot() == original && d.history_counts() == counts,
        "Transform preview changed source/history",
    )?;
    d.commit_state(p.state)?;
    let path = root.join("affine-pixels.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo(),
        "Transform native/one Undo/Redo",
    )?;
    d.undo();
    for (scale, degrees) in [([0., 1.], 0.), ([1., 1.], f64::NAN), ([1e4, 1e4], 0.)] {
        check(
            core::prepare(
                &d,
                key,
                true,
                Transform {
                    scale,
                    degrees,
                    ..Default::default()
                },
                false,
                Filter::Nearest,
            )
            .is_err()
                && d.snapshot() == original,
            "Degenerate/huge pixel transform mutated source",
        )?;
    }
    check(
        core::prepare(&d, key, true, Transform::default(), false, Filter::Bilinear)?.state
            == original,
        "Identity transform resampled feathered pixels",
    )?;
    let p = core::prepare(
        &d,
        key,
        true,
        Transform {
            pivot: [4., 4.],
            degrees: 90.,
            ..Default::default()
        },
        false,
        Filter::Nearest,
    )?;
    check(
        p.state.layers[0].pixel(8, 8, 6, 1) == [220, 20, 30, 255]
            && p.state.layers[0].pixel(8, 8, 6, 2) == [20, 210, 30, 128],
        "Cardinal rotation coordinate oracle",
    )?;
    let mut framed = fixture()?;
    framed.scene.transform = crate::scene::Affine([2., 0., 0., 1., 0., 0.]);
    let p = core::prepare(
        &framed,
        key,
        true,
        Transform {
            translation: [2., 2.],
            ..Default::default()
        },
        false,
        Filter::Nearest,
    )?;
    check(
        p.state.layers[0].pixel(8, 8, 4, 3) == [20, 210, 30, 128]
            && p.state.layers[0].pixel(8, 8, 1, 1) == [220, 20, 30, 255]
            && p.state.scene.selection_coverage([4.5, 3.5]) == 255,
        "Selected transform mixed preserved-page and document coordinates",
    )?;
    let mut fringe = fixture()?;
    fringe.scene.selection = Some(Rect {
        min: [1., 1.],
        max: [2., 2.],
    });
    fringe.layers[0].pixels.fill([0, 255, 0, 0]);
    fringe.layers[0].pixels[9] = [255, 0, 0, 255];
    let p = core::prepare(
        &fringe,
        key,
        true,
        Transform {
            pivot: [1.5, 1.5],
            scale: [2.5; 2],
            degrees: 31.,
            translation: [3., 2.],
            ..Default::default()
        },
        false,
        Filter::Bilinear,
    )?;
    check(
        p.state.layers[0]
            .pixels
            .iter()
            .filter(|p| p[3] > 0)
            .all(|p| p[..3] == [255, 0, 0]),
        "Bilinear transparent corner gained a black/hidden-RGB fringe",
    )?;
    let mut precise = fixture()?;
    precise.color = Default::default();
    let mut high = precise.layers[0]
        .pixels
        .iter()
        .map(|p| crate::color_precision::Sample::from8(*p).rgba16())
        .collect::<Vec<_>>();
    high[45] = [12345, 23456, 34567, 45678];
    precise.layers[0].precise = Some(high.clone());
    precise.layers[0].pixels[45] = crate::color_precision::Sample::from16(high[45]).rgba8();
    let p = core::prepare(&precise, key, true, t, false, Filter::Nearest)?;
    let i = p.state.layers[0].pixel_index(8, 8, 5, 5).unwrap();
    check(
        p.state.layers[0].precise.as_ref().unwrap()[i] == high[45],
        "Unselected native16 sample changed",
    )?;
    let p = core::prepare(
        &d,
        key,
        false,
        Transform {
            pivot: [4., 4.],
            degrees: 45.,
            ..Default::default()
        },
        false,
        Filter::Nearest,
    )?;
    let extent = p.state.layers[0].extent(8, 8);
    check(
        p.state.layers[0].pixel(8, 8, extent.offset[0], extent.offset[1])[3] == 0,
        "Rotated raster bounding corner was not transparent",
    )?;
    let mut vector = fixture()?;
    let layer = vector.scene.lines.add_layer("Vector")?;
    vector.scene.lines.add_path(
        layer,
        vec![[1., 1.], [3., 2.]],
        false,
        2.,
        [10, 20, 30, 128],
    )?;
    let vk = Key::Layer(crate::layer_properties::Target::Vector(layer));
    let p = core::prepare(&vector, vk, false, t, false, Filter::Nearest)?;
    let line = &p.state.scene.lines.layers[0].paths[0];
    check(
        line.points == [[5., 2.], [3., 4.]] && (line.width - 2. * 2f64.sqrt()).abs() < 1e-9,
        "Nonuniform negative vector transform/area width scale",
    )?;
    let (mut material, mk) = crate::material_transform_verify::material()?;
    let n = crate::layer_catalog::source_index(&material, mk).unwrap();
    if let crate::scene::PageObject::Material(m) = &mut material.scene.objects[n] {
        m.repeat = [true, false];
        m.mapping.tiling = crate::material_mapping::Tiling::Reverse;
    }
    let old = material.scene.objects[n].clone();
    let p = core::prepare(&material, mk, false, t, false, Filter::Nearest)?;
    if let (crate::scene::PageObject::Material(a), crate::scene::PageObject::Material(b)) =
        (&old, &p.state.scene.objects[n])
    {
        check(
            a.repeat == b.repeat && a.mapping.tiling == b.mapping.tiling && a.source == b.source,
            "Material transform reset repeat/tiling or bitmap",
        )?;
    } else {
        return Err("Material target disappeared".into());
    }
    let gradient =
        crate::gradient::prepare(&material, 0, Default::default(), true, Default::default())?;
    gradient.restore(&mut material);
    let g = material.scene.gradients.layers[0].clone();
    let p = core::prepare(
        &material,
        Key::Gradient(g.id),
        false,
        t,
        false,
        Filter::Nearest,
    )?;
    check(
        p.state.scene.objects[n] == material.scene.objects[n]
            && p.state.scene.gradients.layers[0].creation != g.creation,
        "Gradient and material at the same collection index were confused",
    )?;
    let mut text = fixture()?;
    text.scene.objects.push(crate::scene::PageObject::Text {
        baseline: [1., 6.],
        text: "A".into(),
        size_px: 4.,
        color: [30, 50, 200, 180],
    });
    let tk = Key::Object(text.scene.catalog.objects.id(0));
    let old = text.snapshot();
    check(
        core::prepare(&text, tk, false, t, false, Filter::Nearest).is_err(),
        "Unsupported editable source silently rasterized without option",
    )?;
    let p = core::prepare(&text, tk, false, t, true, Filter::Nearest)?;
    check(
        matches!(
            p.selected,
            Key::Layer(crate::layer_properties::Target::Raster(_))
        ) && !p.notes.is_empty()
            && text.snapshot() == old,
        "Explicit text conversion transform",
    )?;
    text.commit_state(p.state)?;
    check(
        text.undo() && text.snapshot() == old,
        "Converted source Undo lost editable text",
    )?;
    d.layers[0].locked = true;
    check(
        core::prepare(&d, key, true, t, false, Filter::Nearest).is_err(),
        "Locked pixel transform accepted",
    )?;
    let mut large = Document::create(
        CanvasSpec {
            width: 2048,
            height: 2048,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    large.layers[0].pixels[0] = [10, 20, 30, 255];
    let p = core::prepare(
        &large,
        Key::Layer(crate::layer_properties::raster(&large, 0)),
        false,
        Transform {
            pivot: [1024.; 2],
            scale: [-1., 1.],
            ..Default::default()
        },
        false,
        Filter::Nearest,
    )?;
    check(
        p.state.layers[0].pixel(2048, 2048, 2047, 0) == [10, 20, 30, 255]
            && large.layers[0].pixel(2048, 2048, 0, 0) == [10, 20, 30, 255],
        "4M-pixel layer transform/source preservation",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pivot_scale_rotate_pixels_layers_objects_precision_corners_limits_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}

#[cfg(test)]
mod progress_tests {
    #[test]
    fn cancelling_group_selected_pixels_and_quad_keeps_all_original_data() {
        let d = super::fixture().unwrap();
        let original = d.snapshot();
        let counts = d.history_counts();
        for kind in 0..3 {
            let mut calls = 0usize;
            let mut tick = || {
                calls += 1;
                if calls >= 3 {
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            };
            let result = match kind {
                0 => crate::group_edits::transform_with_progress(
                    &d,
                    &[crate::layer_search::Key::Layer(
                        crate::layer_properties::raster(&d, 0),
                    )],
                    crate::scene::Affine([1.2, 0., 0., 0.8, 1., 1.]),
                    &mut tick,
                )
                .map(|_| ()),
                1 => crate::affine_transform::prepare_with_progress(
                    &d,
                    crate::layer_search::Key::Layer(crate::layer_properties::raster(&d, 0)),
                    true,
                    crate::vector_scope::Transform {
                        scale: [1.2, 0.8],
                        ..Default::default()
                    },
                    false,
                    crate::interpolation::Filter::Bilinear,
                    &mut tick,
                )
                .map(|_| ()),
                _ => crate::deform::prepare_with_progress(
                    &d,
                    &[0],
                    false,
                    crate::deform::Change::Quad(&crate::material_mapping::Warp::Perspective([
                        [0., 0.],
                        [1.1, 0.1],
                        [0.9, 1.],
                        [0., 1.],
                    ])),
                    &mut tick,
                )
                .map(|_| ()),
            };
            assert!(result.is_err() && calls >= 3);
            assert_eq!(d.snapshot(), original);
            assert_eq!(d.history_counts(), counts);
        }
    }
}
