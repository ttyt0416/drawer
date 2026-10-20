use crate::{
    color_precision::Sample,
    color_storage::Config,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
};
fn page(width: u32, height: u32) -> Document {
    Document::create_with_color(
        CanvasSpec {
            width,
            height,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        Config::default(),
    )
    .unwrap()
}
fn put(d: &mut Document, pixels: Vec<[u16; 4]>) {
    d.layers[0].pixels = pixels.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    d.layers[0].precise = Some(pixels);
}
#[test]
fn explicit_profile_flatten_adapter_converts_once_and_one_undo_restores_editable_sources() {
    use crate::{
        color_precision::{Depth, Dither},
        color_profile::{RgbProfile, Transform},
        color_storage::{ProfileChange, prepare, prepare_with_sources},
    };
    let mut d = page(1, 1);
    put(&mut d, vec![[12345, 23456, 34567, 12345]]);
    let id = d.scene.lines.add_layer("Editable source").unwrap();
    d.scene
        .lines
        .add_path(
            id,
            vec![[0., 0.5], [1., 0.5]],
            false,
            1.,
            [200, 20, 30, 128],
        )
        .unwrap();
    let original = d.snapshot();
    let profile = RgbProfile::embedded(
        "P3 target".into(),
        crate::color_precision_verify::display_p3_fixture(),
    )
    .unwrap();
    let config = Config {
        depth: Depth::Eight,
        profile: profile.clone(),
        ..Default::default()
    };
    assert!(prepare(&d, config.clone(), ProfileChange::ConvertPixels).is_err());
    let assigned =
        prepare_with_sources(&d, config.clone(), ProfileChange::AssignNumbers, true).unwrap();
    assert_eq!(assigned.scene.lines, d.scene.lines);
    let source = crate::float_render::Renderer::new(&d)
        .unwrap()
        .pixel([0.5; 2])
        .unwrap();
    let expected = Transform::new(&d.color.profile, &profile)
        .unwrap()
        .map(source)
        .unwrap()
        .store(Depth::Eight, Dither::default(), [0; 2])
        .map(|v| v as u8);
    let converted = prepare_with_sources(&d, config, ProfileChange::ConvertPixels, true).unwrap();
    assert!(converted.scene.lines.layers.is_empty());
    assert_eq!(converted.layers[0].pixels[0], expected);
    assert_eq!(d.snapshot(), original);
    d.commit_state(converted).unwrap();
    assert_eq!(d.history_counts(), (1, 0));
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
    d.scene.lines = Default::default();
    d.scene.appearance.rasters.insert(
        1,
        crate::appearance::Style {
            enabled: true,
            color: Some([150, 80, 200]),
            ..Default::default()
        },
    );
    let profile = RgbProfile::embedded(
        "P3 tint".into(),
        crate::color_precision_verify::display_p3_fixture(),
    )
    .unwrap();
    let config = Config {
        profile,
        ..Default::default()
    };
    assert!(prepare(&d, config.clone(), ProfileChange::ConvertPixels).is_err());
    let converted = prepare_with_sources(&d, config, ProfileChange::ConvertPixels, true).unwrap();
    assert!(converted.scene.appearance.rasters.is_empty());
    let mut eight = page(1, 1);
    eight.color.depth = Depth::Eight;
    eight.layers[0].precise = None;
    let original = eight.snapshot();
    let mut illegal = eight.snapshot();
    illegal.layers[0].pixels[0] = [1, 2, 3, 0];
    assert!(eight.commit_state(illegal).is_err());
    assert_eq!(eight.snapshot(), original);
}
#[test]
fn fixed_seed_dither_does_not_requantize_exact_stored_integers() {
    use crate::color_precision::{Depth, Dither};
    for depth in [Depth::Eight, Depth::Sixteen] {
        for n in 0..=depth.maximum() {
            let sample = Sample::new([n as f64 / depth.maximum() as f64; 4]).unwrap();
            let value = sample.store(
                depth,
                Dither {
                    enabled: true,
                    seed: 0,
                },
                [0; 2],
            );
            assert_eq!(value, [n as u16; 4], "{depth:?} {n}");
        }
    }
}
#[test]
fn float_mask_application_tone_selection_and_multilayer_eraser_keep_native_low_bits() {
    let mut d = page(1, 1);
    let source = [12345, 23456, 34567, 45678];
    put(&mut d, vec![source]);
    d.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 0,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    let original = d.snapshot();
    let tone = crate::soft_proof::Tone {
        gamma: 1.01,
        ..Default::default()
    };
    assert!(crate::soft_proof::apply(&mut d, 0, tone, || false).unwrap());
    let p = source.map(|v| v as f64 / 65535.);
    let weight = 128. / 255.;
    let rgb = [0, 1, 2].map(|n| p[n] * (1. - weight) + p[n].powf(1. / 1.01) * weight);
    assert_eq!(
        d.layers[0].sample_at(0).rgba16(),
        Sample::new([rgb[0], rgb[1], rgb[2], p[3]])
            .unwrap()
            .rgba16()
    );
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
    d.scene.layer_effects.rasters.entry(1).or_default().mask = Some(crate::layer_effects::Mask {
        space: Default::default(),
        coverage: d.scene.selection_mask.clone().unwrap(),
        offset: [0.; 2],
        enabled: true,
        linked: true,
        inverted: false,
    });
    let original = d.snapshot();
    let prepared = crate::layer_masks::prepare(
        &d,
        crate::layer_properties::Target::Raster(1),
        crate::layer_masks::Command::Apply,
    )
    .unwrap();
    let expected = Sample::from16(source).coverage(weight).unwrap().rgba16();
    assert_eq!(prepared.state.layers[0].sample_at(0).rgba16(), expected);
    assert!(
        prepared
            .state
            .scene
            .layer_effects
            .rasters
            .get(&1)
            .is_none_or(|e| e.mask.is_none())
    );
    d.commit_state(prepared.state).unwrap();
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
    d.scene.lines.settings.erase = vec![crate::line_tools::Target::Raster(0)];
    let original = d.snapshot();
    assert!(crate::line_tools::erase(&mut d, [0.5; 2], 1.).unwrap());
    assert_eq!(
        d.layers[0].sample_at(0).rgba16(),
        Sample::from16(source)
            .coverage(1. - weight)
            .unwrap()
            .rgba16()
    );
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
}
#[test]
fn wide_rgb_assets_native_reference_caches_and_psd_have_explicit_srgb_boundaries() {
    use crate::{
        color_profile::{RgbProfile, Transform},
        material_layer::{Image, Layer, Source},
        scene::{Affine, PageObject, Rect},
    };
    let root = crate::test_support::TempDir::new();
    let mut d = page(1, 1);
    d.color.profile = RgbProfile::embedded(
        "P3".into(),
        crate::color_precision_verify::display_p3_fixture(),
    )
    .unwrap();
    let source = [21005, 30007, 11009, 32768];
    put(&mut d, vec![source]);
    let original = d.snapshot();
    let display = Transform::new(&d.color.profile, &RgbProfile::Srgb)
        .unwrap()
        .map(Sample::from16(source))
        .unwrap()
        .rgba8();
    for choice in [
        crate::psd::Choice::RenderedImage,
        crate::psd::Choice::RenderedLayers,
    ] {
        let path = root.0.join(format!("profile-{choice:?}.psd"));
        crate::psd::export(&d, &path, choice, false, DEFAULT_DOCUMENT_BUDGET).unwrap();
        let reopened = crate::psd::open(&path, DEFAULT_DOCUMENT_BUDGET).unwrap();
        assert_eq!(reopened.layers[0].pixels[0], display);
        assert_eq!(d.snapshot(), original);
    }
    let path = root.0.join("wide-native.drawer");
    crate::files::save(&mut d, &path).unwrap();
    let (_, cache) = crate::file_objects::read(&path, None).unwrap();
    assert_eq!(cache.pixels[0], display);
    assert_eq!(d.snapshot(), original);
    d.layers[0].precise = Some(vec![[0; 4]]);
    d.layers[0].pixels = vec![[0; 4]];
    let input = [64, 128, 192, 255];
    d.scene.objects.push(PageObject::Material(Box::new(Layer {
        source: Source::Image(Image {
            width: 1,
            height: 1,
            pixels: std::sync::Arc::new(vec![input]),
        }),
        area: Rect {
            min: [0.; 2],
            max: [1.; 2],
        },
        transform: Affine::default(),
        repeat: [false; 2],
        mapping: Default::default(),
    })));
    let expected = Transform::new(&RgbProfile::Srgb, &d.color.profile)
        .unwrap()
        .map(Sample::from8(input))
        .unwrap()
        .rgba16();
    assert_eq!(
        crate::float_render::Renderer::new(&d)
            .unwrap()
            .pixel([0.5; 2])
            .unwrap()
            .rgba16(),
        expected
    );
    assert_eq!(
        crate::render::Renderer::new(&d)
            .display_sample([0.5; 2])
            .unwrap()
            .rgba8(),
        input
    );
}
#[test]
fn precise_clipboard_round_trip_profile_paste_transfer_and_reject_trailing_bytes() {
    let mut d = page(2, 1);
    let high = vec![[12345, 23456, 34567, 1], [54321, 43210, 32109, 32768]];
    put(&mut d, high.clone());
    let original = d.snapshot();
    let packet = crate::selection_transfer::copy(&d, 0, 10000).unwrap();
    assert_eq!(packet.color.as_ref().unwrap().pixels, high);
    let bytes = packet.encode(10000).unwrap();
    let decoded = crate::paste::ClipboardImage::decode(&bytes, 10000).unwrap();
    assert_eq!(decoded.color, packet.color);
    let state =
        crate::paste::prepare_packet(&d, &decoded, [-2, 3], DEFAULT_DOCUMENT_BUDGET).unwrap();
    assert_eq!(state.layers[1].precise.as_ref().unwrap(), &high);
    d.commit_state(state).unwrap();
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
    for cut in [0, 1, 36, bytes.len() - 1] {
        assert!(crate::paste::ClipboardImage::decode(&bytes[..cut], 10000).is_err());
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(crate::paste::ClipboardImage::decode(&bad, 10000).is_err());
    let mut above = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Upper".into(),
            parent: None,
        },
    )
    .unwrap()
    .state;
    above.layers[1].pixels = high.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    above.layers[1].precise = Some(high.clone());
    above.layers[1].placement = None;
    d.commit_state(above).unwrap();
    let original = d.snapshot();
    let transfer = crate::transfer_down::prepare(&d, d.scene.layer_ids.id(1)).unwrap();
    assert!(transfer.changed > 0);
    assert_eq!(
        transfer.state.layers[1].precise.as_ref().unwrap(),
        &vec![[0; 4]; 2]
    );
    assert!(transfer.state.layers[0].precise.as_ref().unwrap()[0][3] > 1);
    d.commit_state(transfer.state).unwrap();
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
}
#[test]
fn high_precision_fill_selection_move_resize_and_expression_restore_original_channels() {
    let mut d = page(2, 1);
    let values = vec![[12345, 23456, 34567, 123], [54321, 43210, 32109, 456]];
    put(&mut d, values.clone());
    let original = d.snapshot();
    let state = crate::bucket::prepare(
        &d,
        0,
        None,
        [20, 40, 60, 1],
        false,
        false,
        Default::default(),
    )
    .unwrap();
    for (n, p) in state.layers[0].precise.as_ref().unwrap().iter().enumerate() {
        let a = values[n].map(|v| v as f64 / 65535.);
        let b = [20. / 255., 40. / 255., 60. / 255., 1. / 255.];
        let alpha = b[3] + a[3] * (1. - b[3]);
        let rgb = [0, 1, 2].map(|i| (b[i] * b[3] + a[i] * a[3] * (1. - b[3])) / alpha);
        let expected = Sample::new([rgb[0], rgb[1], rgb[2], alpha])
            .unwrap()
            .rgba16();
        assert_eq!(*p, expected);
    }
    d.commit_state(state).unwrap();
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
    for change in [
        crate::sizing::Change::FlipHorizontal,
        crate::sizing::Change::Resample {
            width: 2,
            height: 1,
            dpi: 300.,
            filter: crate::interpolation::Filter::Nearest,
        },
    ] {
        let state = crate::sizing::prepare(
            &original,
            change,
            DEFAULT_DOCUMENT_BUDGET,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            *state.layers[0].precise.as_ref().unwrap(),
            if matches!(change, crate::sizing::Change::FlipHorizontal) {
                vec![values[1], values[0]]
            } else {
                values.clone()
            }
        );
    }
    d.scene.selection = Some(crate::scene::Rect {
        min: [0., 0.],
        max: [1., 1.],
    });
    let selected = d.snapshot();
    let nudge = crate::precision::Nudge::begin(&d, 0, crate::precision::Target::Pixels).unwrap();
    let state = nudge.preview_total(&d, [-2, 1]).unwrap();
    assert_eq!(
        state.layers[0].sample(d.width, d.height, -2, 1).rgba16(),
        values[0]
    );
    assert_eq!(
        state.layers[0].sample(d.width, d.height, 0, 0).rgba16(),
        [0; 4]
    );
    d.commit_state(state).unwrap();
    assert!(d.undo());
    assert_eq!(d.snapshot(), selected);
    let state = crate::expression_color::prepare_pixels(
        &d,
        crate::layer_properties::Target::Raster(1),
        Default::default(),
        crate::layer_search::Expression::Gray,
        Default::default(),
    )
    .unwrap();
    for (n, p) in state.layers[0].precise.as_ref().unwrap().iter().enumerate() {
        let value = Sample::from16(values[n]);
        assert_eq!(
            *p,
            Sample::new([
                value.luminance(),
                value.luminance(),
                value.luminance(),
                value.alpha()
            ])
            .unwrap()
            .rgba16()
        );
    }
    assert_eq!(d.snapshot(), selected);
}
#[test]
fn raster_gradient_has_more_than_256_levels_and_merge_mask_bake_keeps_low_bits() {
    let mut d = page(513, 1);
    let settings = crate::gradient::Settings {
        end: [512.5, 0.5],
        ..Default::default()
    };
    let state = crate::gradient::prepare(&d, 0, settings, false, Default::default()).unwrap();
    let high = state.layers[0].precise.as_ref().unwrap();
    assert_eq!(high[0], [0, 0, 0, 65535]);
    assert_eq!(high[512], [65535; 4]);
    assert!(high.windows(2).all(|p| p[0][0] < p[1][0]));
    d.commit_state(state).unwrap();
    let added = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Other".into(),
            parent: None,
        },
    )
    .unwrap();
    d.commit_state(added.state).unwrap();
    let before = d.snapshot();
    let source = crate::float_render::Renderer::new(&d)
        .unwrap()
        .pixel([100.5, 0.5])
        .unwrap()
        .rgba16();
    let merged = crate::layer_merge::prepare(
        &d,
        crate::layer_merge::Operation::Selected(vec![
            crate::layer_properties::Target::Raster(d.scene.layer_ids.id(0)),
            crate::layer_properties::Target::Raster(d.scene.layer_ids.id(1)),
        ]),
        Default::default(),
    )
    .unwrap();
    d.commit_state(merged.state).unwrap();
    assert_eq!(
        d.layers[0].sample(d.width, d.height, 100, 0).rgba16(),
        source
    );
    assert!(d.undo());
    assert_eq!(d.snapshot(), before);
    let flat = crate::layer_merge::prepare(
        &d,
        crate::layer_merge::Operation::Flatten,
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        flat.state.layers[0]
            .sample(d.width, d.height, 100, 0)
            .rgba16(),
        source
    );
    let raster = crate::raster_conversion::prepare(
        &d,
        crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(
            d.scene.layer_ids.id(0),
        )),
        crate::raster_conversion::Action::Rasterize(crate::layer_search::Expression::Color),
    )
    .unwrap();
    assert_eq!(
        raster.state.layers[0]
            .sample(d.width, d.height, 100, 0)
            .rgba16(),
        source
    );
}
