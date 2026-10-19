use crate::{
    adjustments::{self as a, Destination},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_effects::{Adjustment, ColorModel},
    layer_properties::{self as p, Patch, Target},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make(color: [u8; 4]) -> Result<Document, String> {
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
    d.layers[0].pixels.fill(color);
    Ok(d)
}
fn add(d: &mut Document, value: Adjustment, parent: Option<usize>) -> Result<u64, String> {
    let s = a::prepare(
        d,
        Destination::New {
            name: "Correction".into(),
            parent,
        },
        value,
    )?;
    let id = s.layer;
    d.commit_state(s.state)?;
    Ok(id)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for (value, input, expected) in [
        (
            Adjustment::default(),
            [64, 128, 192, 128],
            [64, 128, 192, 128],
        ),
        (
            Adjustment {
                brightness: 0.2,
                ..Default::default()
            },
            [128, 128, 128, 128],
            [179, 179, 179, 128],
        ),
        (
            Adjustment {
                brightness: -1.,
                ..Default::default()
            },
            [64, 128, 192, 128],
            [0, 0, 0, 128],
        ),
        (
            Adjustment {
                brightness: 1.,
                ..Default::default()
            },
            [64, 128, 192, 128],
            [255, 255, 255, 128],
        ),
        (
            Adjustment {
                contrast: -1.,
                ..Default::default()
            },
            [64, 128, 192, 128],
            [128, 128, 128, 128],
        ),
        (
            Adjustment {
                contrast: 0.5,
                ..Default::default()
            },
            [64, 128, 192, 128],
            [2, 128, 255, 128],
        ),
        (
            Adjustment {
                hue: 120.,
                ..Default::default()
            },
            [255, 0, 0, 128],
            [0, 255, 0, 128],
        ),
        (
            Adjustment {
                saturation: -1.,
                ..Default::default()
            },
            [255, 0, 0, 128],
            [128, 128, 128, 128],
        ),
        (
            Adjustment {
                color_model: ColorModel::Hsv,
                saturation: -1.,
                ..Default::default()
            },
            [255, 0, 0, 128],
            [255, 255, 255, 128],
        ),
    ] {
        let mut d = make(input)?;
        let before = d.snapshot();
        add(&mut d, value, None)?;
        check(
            d.composite_pixel(0, 0) == expected
                && crate::render::Renderer::new(&d).pixel([0.5, 0.5]) == expected
                && d.layers[0] == before.layers[0],
            "Independent correction color/alpha/raw source",
        )?;
        check(
            d.undo() && d.snapshot() == before,
            "Create correction full Undo",
        )?;
    }
    let mut zero = make([128, 20, 60, 128])?;
    zero.layers.push(zero.layers[0].clone());
    zero.layers[1].pixels.fill([40, 200, 10, 64]);
    let before = zero.composite_pixel(0, 0);
    let png1 = root.join("adjust-zero-before.png");
    let png2 = root.join("adjust-zero-after.png");
    crate::png_export::export(&zero, &png1, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    let id = add(&mut zero, Default::default(), None)?;
    crate::png_export::export(&zero, &png2, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        zero.composite_pixel(0, 0) == before
            && std::fs::read(png1).map_err(|e| e.to_string())?
                == std::fs::read(png2).map_err(|e| e.to_string())?,
        "Zero correction must preserve fractional backdrop/PNG",
    )?;
    let state = a::prepare(&zero, Destination::Existing(id), Default::default())?;
    check(
        !zero.commit_state(state.state)?,
        "Unchanged correction edit adds Undo",
    )?;
    let state = a::prepare(
        &zero,
        Destination::Existing(id),
        Adjustment {
            brightness: 0.3,
            ..Default::default()
        },
    )?;
    zero.commit_state(state.state)?;
    check(
        zero.composite_pixel(0, 0)[3] == before[3],
        "Nonzero correction must preserve aggregate backdrop alpha",
    )?;
    let state = p::prepare_patch(
        &zero,
        &[Target::Raster(id)],
        Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    zero.commit_state(state)?;
    check(
        zero.composite_pixel(0, 0) == before,
        "Hidden correction returns original pixels",
    )?;
    let mut d = make([255, 0, 0, 128])?;
    let id = add(
        &mut d,
        Adjustment {
            hue: 120.,
            ..Default::default()
        },
        None,
    )?;
    let source = d.layers[0].clone();
    let mask = crate::layer_folders::rectangle_mask(
        &d,
        crate::scene::Rect {
            min: [0., 0.],
            max: [4., 8.],
        },
        128,
    )?;
    let state = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Mask {
            target: Target::Raster(id),
            mask: Some(mask),
        },
    )?;
    d.commit_state(state.state)?;
    check(
        d.composite_pixel(0, 0) == [127, 128, 0, 128]
            && d.composite_pixel(6, 0) == [255, 0, 0, 128]
            && d.layers[0] == source,
        "Correction mask range/gray strength/source preservation",
    )?;
    let state = p::prepare_patch(
        &d,
        &[Target::Raster(id)],
        Patch {
            opacity: Some(50),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    check(
        d.composite_pixel(0, 0) == [191, 64, 0, 128],
        "Correction mask times layer opacity",
    )?;
    let file = root.join("correction-mask.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Correction parameters/model/mask native reopen",
    )?;
    let count = d.history_counts();
    check(
        !d.edit_pixel(1, 0, 0, [0, 0, 0, 255]) && d.history_counts() == count,
        "Correction pixel paint must be rejected",
    )?;
    let state = p::prepare_patch(
        &d,
        &[Target::Raster(id)],
        Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    let before = d.snapshot();
    check(
        a::prepare(&d, Destination::Existing(id), Default::default()).is_err()
            && d.snapshot() == before,
        "Locked correction edit",
    )?;
    let legacy: Adjustment =
        serde_json::from_str(r#"{"brightness":0.0,"contrast":0.0,"hue":0.0,"saturation":-1.0}"#)
            .map_err(|e| e.to_string())?;
    check(
        legacy.color_model == ColorModel::Hsv
            && legacy.pixel([255, 0, 0, 128]) == [255, 255, 255, 128],
        "Version49 HSV correction semantics preserved",
    )?;
    check(
        a::prepare(
            &d,
            Destination::New {
                name: "Bad".into(),
                parent: None,
            },
            Adjustment {
                brightness: f64::NAN,
                ..Default::default()
            },
        )
        .is_err(),
        "Nonfinite correction rejected",
    )?;
    let mut folder = make([128, 128, 128, 255])?;
    let state = crate::layer_folders::prepare(
        &folder,
        crate::layer_folders::Command::Create {
            name: "Correction scope".into(),
            parent: None,
        },
    )?;
    folder.commit_state(state.state)?;
    add(
        &mut folder,
        Adjustment {
            brightness: 0.2,
            ..Default::default()
        },
        Some(0),
    )?;
    check(
        folder.composite_pixel(0, 0) == [179, 179, 179, 255],
        "Through correction scope",
    )?;
    let state = crate::layer_folders::prepare(
        &folder,
        crate::layer_folders::Command::Mode {
            folder: 0,
            through: false,
        },
    )?;
    folder.commit_state(state.state)?;
    check(
        folder.composite_pixel(0, 0) == [128, 128, 128, 255],
        "Isolated correction scope excludes outside background",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn correction_pixels_alpha_masks_defaults_scopes_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
