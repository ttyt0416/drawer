#[cfg(test)]
mod tests {
    use crate::{
        color_precision::{Depth, Dither, Pixels, Sample},
        color_storage::Config,
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
        float_render::Renderer,
        layer_properties::Target,
    };
    fn page() -> Document {
        Document::create_with_color(
            CanvasSpec {
                width: 1,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config::default(),
        )
        .unwrap()
    }
    fn color(d: &mut Document, n: usize, p: [u16; 4]) {
        d.layers[n].precise = Some(vec![p]);
        d.layers[n].pixels = vec![Sample::from16(p).rgba8()];
    }
    fn add(d: &mut Document, value: [u16; 4]) -> u64 {
        let added = crate::layers::prepare(
            d,
            crate::layers::Command::Create {
                kind: crate::layers::Kind::Raster,
                name: "Float layer".into(),
                parent: None,
            },
        )
        .unwrap();
        let crate::layers::Target::Raster(id) = added.selected else {
            unreachable!()
        };
        d.commit_state(added.state).unwrap();
        let n = crate::layers::index(d, id).unwrap();
        color(d, n, value);
        id
    }
    #[test]
    fn sixteen_bit_float_layers_folders_clipping_correction_and_final_export() {
        let mut d = page();
        let a = [12345, 34567, 56789, 32768];
        let b = [54321, 23456, 45678, 12345];
        color(&mut d, 0, a);
        let upper = add(&mut d, b);
        let expected = Sample::from16(a)
            .over(Sample::from16(b), crate::layer_blend::Mode::Normal, 1.)
            .unwrap();
        let rendered = Renderer::new(&d).unwrap().pixel([0.5; 2]).unwrap();
        assert_eq!(rendered.rgba16(), expected.rgba16());
        assert_eq!(
            Renderer::new(&d)
                .unwrap()
                .output(Depth::Sixteen, Dither::default(), 8)
                .unwrap(),
            Pixels::Sixteen(vec![expected.rgba16()])
        );
        assert_eq!(
            Renderer::new(&d)
                .unwrap()
                .output(Depth::Eight, Dither::default(), 4)
                .unwrap(),
            Pixels::Eight(vec![expected.rgba8()])
        );
        // Compare the full blend/opacity grid to a separate premultiplied float64 formula.
        for mode in crate::layer_blend::Mode::ALL {
            for opacity in [0, 37, 100] {
                d.scene
                    .layer_properties
                    .rasters
                    .entry(upper)
                    .or_default()
                    .blend = mode;
                d.scene
                    .layer_properties
                    .rasters
                    .get_mut(&upper)
                    .unwrap()
                    .opacity = opacity;
                let a = a.map(|v| v as f64 / 65535.);
                let b = b.map(|v| v as f64 / 65535.);
                let sa = b[3] * opacity as f64 / 100.;
                let alpha = sa + a[3] * (1. - sa);
                let blend = mode.rgb([a[0], a[1], a[2]], [b[0], b[1], b[2]]);
                let rgb = [0, 1, 2].map(|i| {
                    ((1. - sa) * a[3] * a[i] + (1. - a[3]) * sa * b[i] + sa * a[3] * blend[i])
                        / alpha
                });
                let expected = Sample::new([rgb[0], rgb[1], rgb[2], alpha])
                    .unwrap()
                    .rgba16();
                let output = Renderer::new(&d).unwrap().pixel([0.5; 2]).unwrap().rgba16();
                assert!(
                    output.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                    "{} / {opacity}",
                    mode.label()
                );
            }
        }
        d.scene.layer_properties.rasters.clear();
        let state =
            crate::clipping::prepare(&d, &[Target::Raster(upper)], None, Some(true)).unwrap();
        d.commit_state(state).unwrap();
        let clipped = Sample::from16(a)
            .atop(Sample::from16(b), crate::layer_blend::Mode::Normal, 1.)
            .unwrap();
        assert_eq!(
            Renderer::new(&d).unwrap().pixel([0.5; 2]).unwrap().rgba16(),
            clipped.rgba16()
        );
        let state =
            crate::clipping::prepare(&d, &[Target::Raster(upper)], None, Some(false)).unwrap();
        d.commit_state(state).unwrap();
        let folder = crate::layer_folders::prepare(
            &d,
            crate::layer_folders::Command::Wrap {
                name: "Float group".into(),
                targets: vec![Target::Raster(1), Target::Raster(upper)],
            },
        )
        .unwrap();
        let folder_target = folder.selected;
        d.commit_state(folder.state).unwrap();
        let Target::Folder(folder) = folder_target else {
            unreachable!()
        };
        d.scene
            .layer_properties
            .folders
            .entry(folder)
            .or_default()
            .opacity = 37;
        assert_eq!(
            Renderer::new(&d).unwrap().pixel([0.5; 2]).unwrap().rgba16(),
            expected.coverage(0.37).unwrap().rgba16()
        );
        // A correction acts on the floating backdrop before the output is quantized.
        let mut corrected = page();
        color(&mut corrected, 0, [12345, 23456, 34567, 32768]);
        let adjustment = crate::layer_effects::Adjustment {
            brightness: 0.00123,
            ..Default::default()
        };
        let prepared = crate::adjustments::prepare(
            &corrected,
            crate::adjustments::Destination::New {
                name: "Float correction".into(),
                parent: None,
            },
            adjustment,
        )
        .unwrap();
        corrected.commit_state(prepared.state).unwrap();
        let input = Sample::from16([12345, 23456, 34567, 32768]);
        let expected = input.adjusted(adjustment).unwrap();
        let output = Renderer::new(&corrected).unwrap().pixel([0.5; 2]).unwrap();
        assert_eq!(output.rgba16(), expected.rgba16());
        assert_ne!(
            output.rgba16(),
            Sample::from8(adjustment.pixel(input.rgba8())).rgba16()
        );
        let original = corrected.snapshot();
        let _ = Renderer::new(&corrected)
            .unwrap()
            .output(Depth::Eight, Dither::default(), 4)
            .unwrap();
        assert_eq!(corrected.snapshot(), original);
        let root = crate::test_support::TempDir::new();
        let png = root.0.join("float-to-png8.png");
        crate::png_export::export(&corrected, &png, 1024 * 1024).unwrap();
        let image = image::ImageReader::open(&png)
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();
        assert_eq!(image.get_pixel(0, 0).0, expected.rgba8());
        assert_eq!(corrected.snapshot(), original);
        assert_eq!(corrected.color.depth, Depth::Sixteen);
        use image::ImageDecoder;
        let mut decoder = image::ImageReader::open(&png)
            .unwrap()
            .into_decoder()
            .unwrap();
        let icc = decoder.icc_profile().unwrap().unwrap();
        assert_eq!(
            moxcms::ColorProfile::new_from_slice(&icc)
                .unwrap()
                .color_space,
            moxcms::DataColorSpace::Rgb
        );
    }
    #[test]
    fn float_editable_gradient_vector_object_world_shadow_and_timeline_sources() {
        use crate::scene::{Camera, PageObject, Rect, WorldMesh};
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 3,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config::default(),
        )
        .unwrap();
        let settings = crate::gradient::Settings {
            start: [0.5, 0.5],
            end: [2.5, 0.5],
            first: [0, 0, 0, 255],
            last: [255; 4],
            ..Default::default()
        };
        let state = crate::gradient::prepare(&d, 0, settings, true, Default::default()).unwrap();
        d.commit_state(state).unwrap();
        let gray = Sample::new([0.5, 0.5, 0.5, 1.]).unwrap();
        assert_eq!(
            Renderer::new(&d)
                .unwrap()
                .pixel([1.5, 0.5])
                .unwrap()
                .rgba16(),
            gray.rgba16()
        );
        assert_ne!(gray.rgba16(), [32896, 32896, 32896, 65535]);
        d.scene.objects.push(PageObject::Line {
            start: [1., 0.5],
            end: [2., 0.5],
            width_px: 1.,
            color: [255, 0, 0, 128],
        });
        let vector = d.scene.lines.add_layer("Float vector").unwrap();
        d.scene
            .lines
            .add_path(
                vector,
                vec![[1.5, 0.], [1.5, 1.]],
                false,
                0.1,
                [0, 0, 255, 64],
            )
            .unwrap();
        d.scene.camera = Some(Camera {
            page_bounds: Rect {
                min: [0.; 2],
                max: [3., 1.],
            },
            position: [0.; 3],
            pixels_per_unit: 1.,
        });
        d.scene.worlds.push(WorldMesh {
            vertices: vec![[0., -0.5, 0.], [0., 0.5, 0.]],
            edges: vec![[0, 1]],
            color: [0, 255, 0, 96],
        });
        let expected = gray
            .over(
                Sample::from8([255, 0, 0, 128]),
                crate::layer_blend::Mode::Normal,
                1.,
            )
            .unwrap()
            .over(
                Sample::from8([0, 0, 255, 64]),
                crate::layer_blend::Mode::Normal,
                1.,
            )
            .unwrap()
            .over(
                Sample::from8([0, 255, 0, 96]),
                crate::layer_blend::Mode::Normal,
                1.,
            )
            .unwrap();
        assert_eq!(
            Renderer::new(&d)
                .unwrap()
                .pixel([1.5, 0.5])
                .unwrap()
                .rgba16(),
            expected.rgba16()
        );
        let root = crate::test_support::TempDir::new();
        let png = root.0.join("float-page-sources.png");
        crate::png_export::export(&d, &png, 1024 * 1024).unwrap();
        assert_eq!(
            image::ImageReader::open(png)
                .unwrap()
                .decode()
                .unwrap()
                .into_rgba8()
                .get_pixel(1, 0)
                .0,
            expected.rgba8()
        );
        let mut shadow = page();
        shadow.scene.objects.push(PageObject::Shadow {
            bounds: Rect {
                min: [-1., -1.],
                max: [0.25, 1.],
            },
            offset: [0.; 2],
            radius_px: 1.,
            color: [200, 20, 30, 127],
        });
        let expected = Sample::from8([200, 20, 30, 127]).coverage(0.75).unwrap();
        assert_eq!(
            Renderer::new(&shadow)
                .unwrap()
                .pixel([0.5; 2])
                .unwrap()
                .rgba16(),
            expected.rgba16()
        );
        assert_ne!(expected.rgba16(), Sample::from8(expected.rgba8()).rgba16());
        let mut animation = page();
        animation.animation = Some(crate::animation::Timeline {
            fps: Default::default(),
            frames: 1,
            tracks: vec![crate::animation::Track {
                name: "Float track".into(),
                cells: vec![crate::animation::Cell {
                    name: "Cell".into(),
                    width: 1,
                    height: 1,
                    offset: [0; 2],
                    pixels: std::sync::Arc::new(vec![[150, 50, 100, 128]]),
                }],
                exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
                end: 1,
            }],
            audio: vec![],
        });
        animation
            .scene
            .layer_properties
            .tracks
            .entry(0)
            .or_default()
            .opacity = 37;
        let expected = Sample::from8([150, 50, 100, 128]).coverage(0.37).unwrap();
        assert_eq!(
            Renderer::new(&animation)
                .unwrap()
                .pixel([0.5; 2])
                .unwrap()
                .rgba16(),
            expected.rgba16()
        );
    }
    #[test]
    fn float_border_and_watercolor_edge_caches_keep_original_channels_and_fractional_alpha() {
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 3,
                height: 3,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config::default(),
        )
        .unwrap();
        let source = [12345, 23456, 34567, 23456];
        let mut high = vec![[0; 4]; 9];
        high[4] = source;
        d.layers[0].pixels = high.iter().map(|p| Sample::from16(*p).rgba8()).collect();
        d.layers[0].precise = Some(high);
        let style = crate::appearance::Style {
            enabled: true,
            border: 1,
            border_color: [20, 40, 60, 128],
            rim: 0.25,
            ..Default::default()
        };
        let state = crate::appearance::prepare(&d, Target::Raster(1), style).unwrap();
        d.commit_state(state).unwrap();
        let original = d.snapshot();
        let renderer = Renderer::new(&d).unwrap();
        let raw = Sample::from16(source);
        let ink = raw.with_rgb(raw.rgb().map(|v| v * 0.75)).unwrap();
        assert!(
            renderer
                .pixel([1.5; 2])
                .unwrap()
                .rgba16()
                .iter()
                .zip(ink.rgba16())
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
        let border = Sample::from8([20, 40, 60, 128])
            .coverage(raw.alpha())
            .unwrap();
        assert!(
            renderer
                .pixel([0.5; 2])
                .unwrap()
                .rgba16()
                .iter()
                .zip(border.rgba16())
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
        assert_eq!(d.snapshot(), original);
        let mut vector = Document::create_with_color(
            CanvasSpec {
                width: 8,
                height: 8,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config::default(),
        )
        .unwrap();
        let id = vector
            .scene
            .lines
            .add_layer("Float display vector")
            .unwrap();
        vector
            .scene
            .lines
            .add_path(
                id,
                vec![[2.5, 3.5], [4.5, 3.5]],
                false,
                1.,
                [200, 100, 50, 128],
            )
            .unwrap();
        let state = crate::appearance::prepare(
            &vector,
            Target::Vector(id),
            crate::appearance::Style {
                enabled: true,
                border: 1,
                rim: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
        vector.commit_state(state).unwrap();
        let original = vector.snapshot();
        let renderer = Renderer::new(&vector).unwrap();
        let expected = Sample::from8([100, 50, 25, 128]).rgba16();
        assert!(
            renderer
                .pixel([3.5; 2])
                .unwrap()
                .rgba16()
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
        assert!(
            renderer
                .pixel([1.5, 3.5])
                .unwrap()
                .rgba16()
                .iter()
                .zip([0, 0, 0, 32896])
                .all(|(a, b)| a.abs_diff(b) <= 1)
        );
        assert_eq!(vector.snapshot(), original);
    }
}
