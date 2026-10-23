use super::local_inference::*;
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, Rect},
};
use std::sync::atomic::{AtomicBool, Ordering};
#[test]
fn snapshot_selection_coordinates_empty_alpha_freeze_cancel_and_transfer_limits() {
    let root = crate::test_support::TempDir::new();
    let mut d = Document::create(
        CanvasSpec {
            width: 2,
            height: 2,
            dpi: 300.,
            background: Background::Color([200, 80, 30]),
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .unwrap();
    d.layers[0].pixels[3] = [1, 2, 3, 0];
    let original = d.snapshot();
    let c = AtomicBool::new(false);
    assert!(snapshot(&d, true, &c).is_err());
    let frozen = snapshot(&d, false, &c).unwrap();
    assert_eq!(frozen.pixels[3], [0; 4]);
    assert_eq!(d.snapshot(), original);
    d.layers[0].pixels[0] = [0; 4];
    assert_eq!(frozen.pixels[0], [200, 80, 30, 255]);
    d.layers[0].pixels[0] = [200, 80, 30, 255];
    d.scene.transform = Affine([1., 0., 0., 1., 1., 0.]);
    d.scene.selection = Some(Rect {
        min: [0., 0.],
        max: [1., 2.],
    });
    let selected = snapshot(&d, true, &c).unwrap();
    assert_eq!(selected.pixels[0], [0; 4]);
    assert_eq!(selected.pixels[1], [200, 80, 30, 255]);
    d.scene.selection = None;
    d.scene.selection_mask = Some(crate::selection::Mask { runs: vec![] });
    assert!(
        snapshot(&d, true, &c)
            .unwrap()
            .pixels
            .iter()
            .all(|p| p[3] == 0)
    );
    c.store(true, Ordering::Relaxed);
    assert!(snapshot(&d, false, &c).is_err());
    let path = root.0.join("pixels.rgba");
    write_image(&path, &frozen, MAX_INPUT).unwrap();
    assert_eq!(read_image(&path, MAX_INPUT).unwrap(), frozen);
    assert!(read_image(&path, 1).is_err());
    std::fs::write(&path, [255u8; 8]).unwrap();
    assert!(read_image(&path, MAX_INPUT).is_err());
}

#[test]
fn new_inference_layer_preserves_locked_high_precision_source_and_uses_document_profile() {
    let color = crate::color_storage::Config {
        depth: crate::color_precision::Depth::Sixteen,
        profile: crate::color_profile::RgbProfile::embedded(
            "Display P3".into(),
            moxcms::ColorProfile::new_display_p3().encode().unwrap(),
        )
        .unwrap(),
        ..Default::default()
    };
    let mut d = Document::create_with_color(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        color,
    )
    .unwrap();
    let raw = [12345, 23456, 34567, 1];
    d.layers[0].pixels[0] = crate::color_precision::Sample::from16(raw).rgba8();
    d.layers[0].precise = Some(vec![raw]);
    d.layers[0].locked = true;
    let input = crate::references::LocalImage {
        width: 1,
        height: 1,
        pixels: vec![[200, 80, 20, 128]],
    };
    let original = d.snapshot();
    let result = Output {
        task: Task::Denoise,
        image: Some(input.clone()),
        points: vec![],
        report: String::new(),
    };
    let state = adopt(&d, &input, &result, 0.5, false).unwrap();
    assert_eq!(state.layers[0], original.layers[0]);
    assert_eq!(state.color, original.color);
    assert_eq!(state.layers[1].precise.as_ref().unwrap()[0][3], 128 * 257);
    let new = crate::color_precision::Sample::from16(state.layers[1].precise.as_ref().unwrap()[0]);
    let displayed = crate::color_profile::WorkingSpace::new(&d.color.profile)
        .unwrap()
        .to_srgb(new)
        .unwrap()
        .rgba8();
    assert!(
        displayed
            .iter()
            .zip(input.pixels[0])
            .all(|(a, b)| a.abs_diff(b) <= 1)
    );
    d.commit_state(state).unwrap();
    assert!(d.undo());
    assert_eq!(d.snapshot(), original);
}
