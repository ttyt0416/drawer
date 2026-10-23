use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    material_layer::{self as core, Image, Layer, Source},
    metal_material::Metal,
    scene::{Affine, PageObject, Rect},
};
use std::{path::Path, sync::Arc};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn texture() -> Image {
    Image {
        width: 2,
        height: 2,
        pixels: Arc::new(vec![
            [64, 64, 64, 255],
            [192, 192, 192, 128],
            [250, 50, 90, 0],
            [120, 120, 120, 1],
        ]),
    }
}
pub fn fixture() -> Layer {
    Layer {
        source: Source::Metal(Box::new(Metal {
            color: [200, 100, 50],
            texture: Some(texture()),
            direction: 0.,
            ..Default::default()
        })),
        area: Rect {
            min: [0.; 2],
            max: [4.; 2],
        },
        transform: Affine::default(),
        repeat: [false; 2],
        mapping: Default::default(),
    }
}
pub fn verify(root: &Path) -> Result<(), String> {
    let mut layer = fixture();
    let original_layer = layer.clone();
    let mut hidden_changed = layer.clone();
    if let Source::Metal(m) = &mut hidden_changed.source {
        Arc::make_mut(&mut m.texture.as_mut().unwrap().pixels)[2] = [0, 0, 0, 0];
    }
    check(
        hidden_changed.pixel([0.5, 0.5]) == layer.pixel([0.5, 0.5]),
        "Hidden transparent texture RGB must not affect relief",
    )?;
    check(
        layer.valid(false) && layer.pixel([0.5, 0.5]) == [60, 30, 15, 255],
        "Independent metallic diffuse formula",
    )?;
    check(
        layer.pixel([0.5, 1.5]) == [0; 4]
            && layer.pixel([1.5, 1.5])[3] == 1
            && layer.pixel([1.5, 0.5])[3] == 128
            && layer.pixel([3.5, 0.5]) == [0; 4],
        "Metallic transparent edge/low alpha/bounds",
    )?;
    if let Source::Metal(m) = &mut layer.source {
        m.strength = 0.;
    }
    check(
        layer.pixel([0.5, 0.5]) == [64, 64, 64, 255]
            && layer.pixel([1.5, 0.5]) == [192, 192, 192, 128],
        "Strength zero must preserve raw visible texture bytes",
    )?;
    layer = original_layer.clone();
    if let Source::Metal(m) = &mut layer.source {
        m.direction = 180.;
        m.gloss = 0.;
    }
    check(
        layer.pixel([0.5, 0.5]) == [200, 100, 50, 255],
        "Light direction and matte metallic limit",
    )?;
    if let Source::Metal(m) = &mut layer.source {
        m.gloss = 1.;
    }
    check(
        layer.pixel([0.5, 0.5]) != [200, 100, 50, 255],
        "Gloss must affect specular output",
    )?;
    let mut missing = Metal::default();
    let flat = missing.pixel([2.5, 3.5], |_| None);
    check(
        flat[3] == 255,
        "Procedural fallback is available without assets",
    )?;
    missing.texture = Some(Image {
        pixels: Arc::new(vec![]),
        ..texture()
    });
    check(
        missing.valid(true) && !missing.valid(false) && missing.pixel([2.5, 3.5], |_| None) == flat,
        "A missing in-memory texture uses the same fallback; incomplete native images remain invalid",
    )?;
    missing.texture = None;
    for bad in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        missing.strength = bad;
        check(!missing.valid(false), "Invalid metallic strength accepted")?;
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 4,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let original = d.snapshot();
    let counts = d.history_counts();
    let (state, id) = core::prepare(&d, None, "Local metallic foil", original_layer.clone())?;
    check(d.snapshot() == original, "Metal preview modified document")?;
    d.commit_state(state)?;
    let renderer = crate::render::Renderer::new(&d);
    let pixels: Vec<_> = (0..4)
        .flat_map(|y| (0..4).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
        .map(|p| renderer.pixel(p))
        .collect();
    check(
        pixels[0] == [60, 30, 15, 255]
            && pixels[4] == [0; 4]
            && d.layers == original.layers
            && d.history_counts().0 == counts.0 + 1,
        "Metal composition/one Undo/source preservation",
    )?;
    let path = root.join("metallic-material.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot(),
        "Metal parameters and embedded texture native mismatch",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&78u32.to_le_bytes());
    let old = root.join("metallic-old.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Older native header accepted metallic data",
    )?;
    let plan = crate::raster_conversion::prepare(
        &d,
        crate::layer_search::Key::Object(id),
        crate::raster_conversion::Action::Rasterize(crate::layer_search::Expression::Color),
    )?;
    let raster = crate::layer_merge::preview(&plan.state, d.current_frame)?;
    let r = crate::render::Renderer::new(&raster);
    for (y, row) in pixels.chunks(4).enumerate() {
        for (x, p) in row.iter().enumerate() {
            check(
                r.pixel([x as f64 + 0.5, y as f64 + 0.5]) == *p,
                "Metal raster output differs",
            )?;
        }
    }
    check(
        d.undo() && d.snapshot() == original,
        "Metal Undo did not restore original",
    )?;
    // Tiling and transforms stay editable and do not rewrite the texture.
    let mut tiled = original_layer.clone();
    tiled.repeat = [true; 2];
    tiled.transform = Affine([1., 0., 0., 1., 1., 1.]);
    let mut reference = original_layer.clone();
    reference.repeat = [true; 2];
    check(
        tiled.pixel([3.5, 1.5]) == reference.pixel([0.5, 0.5]),
        "Metal texture tiling/transformed coordinates",
    )?;
    if let Source::Metal(m) = &mut tiled.source {
        m.texture = None;
    }
    let (state, _) = core::prepare(&d, None, "Fallback foil", tiled)?;
    check(
        matches!(&state.scene.objects[0],PageObject::Material(m)if matches!(&m.source,Source::Metal(s)if s.texture.is_none())),
        "Procedural source parameters lost",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metallic_material_math_alpha_texture_fallback_native_raster_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
    #[test]
    fn metallic_material_is_color_managed_and_preserves_sixteen_bit_original() {
        use crate::color_precision::{Depth, Sample};
        let config = crate::color_storage::Config {
            depth: Depth::Sixteen,
            profile: crate::color_profile::RgbProfile::embedded(
                "Display P3".into(),
                moxcms::ColorProfile::new_display_p3().encode().unwrap(),
            )
            .unwrap(),
            ..Default::default()
        };
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 4,
                height: 4,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            config,
        )
        .unwrap();
        let raw = [12345, 23456, 34567, 1];
        d.layers[0].pixels[15] = Sample::from16(raw).rgba8();
        d.layers[0].precise.as_mut().unwrap()[15] = raw;
        let original = d.snapshot();
        let (state, _) = core::prepare(&d, None, "Managed metal", fixture()).unwrap();
        d.commit_state(state).unwrap();
        assert_eq!(d.layers[0], original.layers[0]);
        let r = crate::float_render::Renderer::new(&d).unwrap();
        let actual = r.display_sample([0.5, 0.5]).unwrap().rgba8();
        assert!(
            actual
                .iter()
                .zip([60, 30, 15, 255])
                .all(|(a, b)| a.abs_diff(b) <= 1),
            "{actual:?}"
        );
        assert_eq!(r.display_sample([1.5, 1.5]).unwrap().rgba16()[3], 257);
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
    }
}
