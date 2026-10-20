#[cfg(test)]
mod tests {
    use crate::{
        color_precision::{Depth, Sample},
        color_profile::RgbProfile,
        color_storage::{Config, ProfileChange, prepare},
        document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    };
    #[test]
    fn high_precision_pixel_input_geometry_proof_and_linked_filter_keep_low_bits() {
        use crate::{group_edits as edits, layer_properties::Target, layer_search::Key};
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Config::default(),
        )
        .unwrap();
        let source = vec![
            [12345, 23456, 34567, 45678],
            [1, 2, 3, 4],
            [1001, 2002, 3003, 4004],
            [55555, 44444, 33333, 22222],
        ];
        d.layers[0].pixels = source.iter().map(|v| Sample::from16(*v).rgba8()).collect();
        d.layers[0].precise = Some(source.clone());
        let before = d.snapshot();
        assert!(d.edit_pixel(0, 0, 0, [10, 20, 30, 128]));
        assert_eq!(
            d.layers[0].precise.as_ref().unwrap()[0],
            [2570, 5140, 7710, 32896]
        );
        assert!(d.undo());
        assert_eq!(d.snapshot(), before);
        let key = Key::Layer(Target::Raster(1));
        d.scene.clipping.alpha.insert(Target::Raster(1));
        let matrix = edits::matrix([1.; 2], 90., 1., false).unwrap();
        let state = edits::transform(&d, &[key], matrix).unwrap();
        d.commit_state(state).unwrap();
        let rotated = vec![source[2], source[0], source[3], source[1]];
        assert_eq!(d.layers[0].precise.as_ref().unwrap(), &rotated);
        let mut illegal = d.snapshot();
        illegal.layers[0].precise.as_mut().unwrap()[0][3] += 1;
        illegal.layers[0].pixels[0] =
            Sample::from16(illegal.layers[0].precise.as_ref().unwrap()[0]).rgba8();
        assert!(d.commit_state(illegal).is_err());
        let filter = crate::layer_effects::Adjustment {
            brightness: 0.0003,
            ..Default::default()
        };
        let state = edits::filter(&d, &[key], filter).unwrap();
        let original = d.snapshot();
        d.commit_state(state).unwrap();
        let expected = rotated
            .iter()
            .map(|p| Sample::from16(*p).adjusted(filter).unwrap().rgba16())
            .collect::<Vec<_>>();
        assert_eq!(d.layers[0].precise.as_ref().unwrap(), &expected);
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
    }
    #[test]
    fn native_16bit_low_bits_profile_dither_conversion_undo_and_bad_payloads() {
        let root = crate::test_support::TempDir::new();
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
        assert_eq!(d.color.depth, Depth::Sixteen);
        let high = vec![[12345, 34567, 56789, 32768], [1, 2, 3, 65535], [0; 4]];
        d.layers[0].pixels = high.iter().map(|p| Sample::from16(*p).rgba8()).collect();
        d.layers[0].precise = Some(high.clone());
        let original = d.snapshot();
        let file = root.0.join("precision16.drawer");
        crate::files::save(&mut d, &file).unwrap();
        let opened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).unwrap();
        assert_eq!(opened.snapshot(), original);
        assert_eq!(opened.layers[0].precise.as_ref(), Some(&high));
        let mut config = d.color.clone();
        config.depth = Depth::Eight;
        let preview = prepare(&d, config, ProfileChange::AssignNumbers).unwrap();
        assert_eq!(d.snapshot(), original);
        assert_eq!(preview.color.depth, Depth::Eight);
        d.commit_state(preview).unwrap();
        let eight = d.snapshot();
        assert!(d.layers[0].precise.is_none());
        assert_eq!(d.layers[0].pixels[0], Sample::from16(high[0]).rgba8());
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
        assert!(d.redo());
        assert_eq!(d.snapshot(), eight);
        assert!(d.undo());
        let profile = RgbProfile::embedded(
            "Display P3".into(),
            moxcms::ColorProfile::new_display_p3().encode().unwrap(),
        )
        .unwrap();
        let config = Config {
            profile,
            ..d.color.clone()
        };
        let assigned = prepare(&d, config.clone(), ProfileChange::AssignNumbers).unwrap();
        assert_eq!(assigned.layers, d.layers);
        assert_ne!(assigned.color.profile, d.color.profile);
        d.commit_state(assigned).unwrap();
        let assigned = d.snapshot();
        crate::files::save(&mut d, &file).unwrap();
        assert_eq!(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .unwrap()
                .snapshot(),
            assigned
        );
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
        let converted = prepare(&d, config, ProfileChange::ConvertPixels).unwrap();
        assert_ne!(converted.layers[0].precise, Some(high.clone()));
        assert_eq!(converted.layers[0].precise.as_ref().unwrap()[0][3], 32768);
        d.commit_state(converted).unwrap();
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
        let mut config = d.color.clone();
        config.depth = Depth::Eight;
        config.dither.enabled = true;
        config.dither.seed = 314159;
        let a = prepare(&d, config.clone(), ProfileChange::AssignNumbers).unwrap();
        let b = prepare(&d, config, ProfileChange::AssignNumbers).unwrap();
        assert_eq!(a, b);
        d.commit_state(a).unwrap();
        let quantized = d.snapshot();
        crate::files::save(&mut d, &file).unwrap();
        assert_eq!(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .unwrap()
                .snapshot(),
            quantized
        );
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
        crate::files::save(&mut d, &file).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        let mut old = bytes.clone();
        old[8..12].copy_from_slice(&61u32.to_le_bytes());
        let path = root.0.join("precision-old.drawer");
        std::fs::write(&path, old).unwrap();
        assert!(crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).is_err());
        let path = root.0.join("precision-truncated.drawer");
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).is_err());
        d.layers[0].pixels[0][0] ^= 1;
        assert!(crate::files::save(&mut d, &file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
}
