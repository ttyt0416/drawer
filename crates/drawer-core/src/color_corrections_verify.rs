use crate::{
    color_corrections::{self as core, Operation as O},
    color_precision::Sample,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_effects::Adjustment,
    scene::Rect,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let row = [
        [0, 0, 0, 255],
        [255; 4],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [128, 128, 128, 128],
        [25, 90, 200, 17],
        [11, 22, 33, 0],
    ];
    d.layers[0].pixels = row.into_iter().chain(row).collect();
    Ok(d)
}
fn add(d: &mut Document) -> Result<u64, String> {
    let p = crate::adjustments::prepare(
        d,
        crate::adjustments::Destination::New {
            name: "Invert RGB".into(),
            parent: None,
        },
        Adjustment {
            invert: true,
            ..Default::default()
        },
    )?;
    let id = p.layer;
    d.commit_state(p.state)?;
    Ok(id)
}
pub fn math() -> Result<(), String> {
    for op in O::defaults() {
        check(op.valid(), "Default correction invalid")?;
    }
    check(
        !O::Levels {
            input: [0.8, 0.2],
            output: [0., 1.],
            gamma: 1.,
        }
        .valid()
            && !O::Levels {
                input: [0., 1.],
                output: [1., 0.],
                gamma: 1.,
            }
            .valid()
            && !O::Posterize(1).valid()
            && !O::Threshold(f64::NAN).valid(),
        "Invalid black/white points, posterize count or threshold accepted",
    )?;
    check(
        O::Levels {
            input: [0.25, 0.75],
            output: [0.1, 0.9],
            gamma: 1.,
        }
        .rgb([0., 0.5, 1.])?
            == [0.1, 0.5, 0.9],
        "Levels independent midpoint/endpoints",
    )?;
    check(
        O::Balance([0.1, -0.2, 1.])
            .rgb([0.2, 0.5, 0.9])?
            .iter()
            .zip([0.3, 0.3, 1.])
            .all(|(a, b)| (*a - b).abs() < 1e-12),
        "Balance independent offsets/clamp",
    )?;
    check(
        O::Posterize(2).rgb([0.49, 0.5, 1.])? == [0., 1., 1.]
            && O::Posterize(3).rgb([0., 0.5, 1.])? == [0., 0.5, 1.],
        "Posterize minimum2 or discrete bins",
    )?;
    check(
        O::Threshold(0.).rgb([0.; 3])? == [1.; 3]
            && O::Threshold(1.).rgb([1.; 3])? == [1.; 3]
            && O::Threshold(1.).rgb([0.999; 3])? == [0.; 3],
        "Threshold0/1 endpoints",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    math()?;
    let mut d = fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    for identity in [O::defaults()[0], O::Balance([0.; 3])] {
        check(
            core::prepare(&d, 0, false, identity)? == original,
            "Identity correction changed stored pixels",
        )?;
    }
    let state = core::prepare(&d, 0, false, O::Invert)?;
    for (old, new) in d.layers[0].pixels.iter().zip(&state.layers[0].pixels) {
        let expected = if old[3] == 0 {
            *old
        } else {
            [255 - old[0], 255 - old[1], 255 - old[2], old[3]]
        };
        check(
            *new == expected,
            "Black/white/primary/gray/transparent invert or alpha preservation",
        )?;
    }
    check(
        d.snapshot() == original && d.history_counts() == counts,
        "Correction preview changed original",
    )?;
    d.commit_state(state)?;
    d.commit_state(core::prepare(&d, 0, false, O::Invert)?)?;
    check(
        d.snapshot() == original,
        "Two pixel inversions failed to restore original bytes",
    )?;
    d.scene.selection = Some(Rect {
        min: [0., 0.],
        max: [4., 1.],
    });
    let state = core::prepare(&d, 0, true, O::Invert)?;
    check(
        state.layers[0].pixels[0] == [255; 4]
            && state.layers[0].pixels[4] == d.layers[0].pixels[4]
            && state.layers[0].pixels[8] == d.layers[0].pixels[8]
            && state.scene == d.scene,
        "Selection outside/geometry changed",
    )?;
    d.scene.selection = None;
    for op in O::defaults() {
        let state = core::prepare(&d, 0, false, op)?;
        check(
            state.layers[0]
                .pixels
                .iter()
                .zip(&d.layers[0].pixels)
                .all(|(a, b)| a[3] == b[3]),
            "Correction changed alpha",
        )?;
    }
    let mut faint = fixture()?;
    faint.color = Default::default();
    let mut high = vec![[0; 4]; 16];
    high[0] = [12345, 23456, 34567, 10];
    faint.layers[0].pixels = high.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    faint.layers[0].precise = Some(high.clone());
    let state = core::prepare(&faint, 0, false, O::Invert)?;
    check(
        state.layers[0].precise.as_ref().unwrap()[0] == [53190, 42079, 30968, 10]
            && state.layers[0].pixels[0][3] == 0,
        "Native16 inversion lost low bits or faint alpha",
    )?;
    faint.commit_state(state)?;
    faint.commit_state(core::prepare(&faint, 0, false, O::Invert)?)?;
    check(
        faint.layers[0].precise.as_ref() == Some(&high),
        "Two native16 inversions changed storage",
    )?;
    let mut layer = fixture()?;
    let source = layer.layers[0].clone();
    let before = layer.snapshot();
    let count = layer.history_counts();
    let id = add(&mut layer)?;
    check(
        layer.layers[0] == source
            && layer.history_counts().0 == count.0 + 1
            && layer.composite_pixel(0, 0) == [255; 4]
            && layer.composite_pixel(2, 0) == [0, 255, 255, 255],
        "Nondestructive inversion source/alpha/one Undo",
    )?;
    let mask = crate::layer_folders::rectangle_mask(
        &layer,
        Rect {
            min: [0.; 2],
            max: [4., 2.],
        },
        128,
    )?;
    let p = crate::layer_folders::prepare(
        &layer,
        crate::layer_folders::Command::Mask {
            target: crate::layer_properties::Target::Raster(id),
            mask: Some(mask),
        },
    )?;
    layer.commit_state(p.state)?;
    check(
        layer.composite_pixel(0, 0)[0].abs_diff(128) <= 1
            && layer.composite_pixel(4, 0) == source.pixels[4],
        "Inversion correction mask range/strength",
    )?;
    let s = crate::layer_properties::prepare_patch(
        &layer,
        &[crate::layer_properties::Target::Raster(id)],
        crate::layer_properties::Patch {
            opacity: Some(50),
            ..Default::default()
        },
    )?;
    layer.commit_state(s)?;
    check(
        layer.composite_pixel(0, 0)[0].abs_diff(64) <= 1
            && layer.composite_pixel(0, 0)[3] == 255
            && layer.layers[0] == source,
        "Inversion correction opacity/source/alpha",
    )?;
    let saved = layer.snapshot();
    let file = root.join("inversion-v69.drawer");
    crate::files::save(&mut layer, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == saved,
        "Inversion parameters/mask/native round trip",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    check(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()) == 69,
        "Inversion document version",
    )?;
    bytes[8..12].copy_from_slice(&68u32.to_le_bytes());
    let old = root.join("downgraded-inversion.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Inversion accepted below v69",
    )?;
    while layer.history_counts().0 > count.0 {
        layer.undo();
    }
    check(
        layer.snapshot() == before,
        "Inversion Undo did not restore source document",
    )?;
    let mut profile = fixture()?;
    profile.color = Default::default();
    profile.color.profile = crate::color_profile::RgbProfile::embedded(
        "Display P3".into(),
        moxcms::ColorProfile::new_display_p3()
            .encode()
            .map_err(|e| e.to_string())?,
    )?;
    let mut high = vec![[0; 4]; 16];
    high[0] = [10000, 20000, 30000, 32768];
    profile.layers[0].pixels = high.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    profile.layers[0].precise = Some(high);
    add(&mut profile)?;
    let value = crate::float_render::Renderer::new(&profile)?
        .pixel([0.5; 2])?
        .rgba16();
    check(
        value
            .iter()
            .zip([55535, 45535, 35535, 32768])
            .all(|(a, b)| a.abs_diff(b) <= 1),
        "Live inversion used display-sRGB instead of document RGB",
    )?;
    let state = crate::color_storage::prepare(
        &profile,
        crate::color_storage::Config {
            profile: crate::color_profile::RgbProfile::Srgb,
            ..profile.color.clone()
        },
        crate::color_storage::ProfileChange::AssignNumbers,
    )?;
    check(
        state.layers == profile.layers && state.scene.layer_effects == profile.scene.layer_effects,
        "Assigning profile changed source or inversion parameters",
    )?;
    let linked = crate::group_edits::filter(
        &profile,
        &[crate::layer_search::Key::Layer(
            crate::layer_properties::raster(&profile, 0),
        )],
        Adjustment {
            invert: true,
            ..Default::default()
        },
    )?;
    check(
        linked.layers[0].precise.as_ref().unwrap()[0] == [55535, 45535, 35535, 32768],
        "Linked raster inversion did not use document RGB",
    )?;
    let legacy: Adjustment =
        serde_json::from_str(r#"{"brightness":0.0,"contrast":0.0,"hue":0.0,"saturation":0.0}"#)
            .map_err(|e| e.to_string())?;
    check(!legacy.invert, "Legacy correction gained inversion")?;
    let config = crate::color_storage::Config {
        profile: crate::color_profile::RgbProfile::Srgb,
        ..profile.color.clone()
    };
    check(
        crate::color_storage::prepare(
            &profile,
            config.clone(),
            crate::color_storage::ProfileChange::ConvertPixels,
        )
        .is_err(),
        "Live correction profile conversion silently reinterpreted inversion",
    )?;
    let expected = crate::color_profile::Transform::new(&profile.color.profile, &config.profile)?
        .map(crate::float_render::Renderer::new(&profile)?.pixel([0.5; 2])?)?
        .rgba16();
    let flattened = crate::color_storage::prepare_with_sources(
        &profile,
        config,
        crate::color_storage::ProfileChange::ConvertPixels,
        true,
    )?;
    let flat = crate::layer_merge::preview(&flattened, profile.current_frame)?;
    let actual = crate::float_render::Renderer::new(&flat)?
        .pixel([0.5; 2])?
        .rgba16();
    check(
        flat.layers.len() == 1
            && !crate::layer_effects::requires_v69(&flat.scene)
            && actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
        "Explicit flatten/profile conversion changed visible inversion",
    )?;
    let legacy_file = root.join("plain-v68.drawer");
    let mut plain = fixture()?;
    crate::files::save(&mut plain, &legacy_file).map_err(|e| e.to_string())?;
    let mut bytes = std::fs::read(&legacy_file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&68u32.to_le_bytes());
    std::fs::write(&legacy_file, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&legacy_file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == plain.snapshot(),
        "Old v68 without inversion no longer opens",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn levels_balance_posterize_threshold_bounds() {
        super::math().unwrap();
    }
    #[test]
    fn rgb_invert_selection_precision_layers_mask_profile_native_history() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
