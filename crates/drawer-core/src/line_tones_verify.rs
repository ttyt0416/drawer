use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    line_tones::{self as core, *},
    model_library::{self as lib, Placement, Shape},
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn prep(d: &Document, s: Settings) -> Result<Prepared, String> {
    core::prepare(d, s, B, &AtomicBool::new(false))
}
pub fn image(checker: bool) -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    for y in 0..64 {
        for x in 0..64 {
            let color = if checker {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                [v, v, v, 255]
            } else if (16..48).contains(&x) && (16..48).contains(&y) {
                [188, 188, 188, 255]
            } else {
                [255, 0, 255, 0]
            };
            d.layers[0].pixels[y * 64 + x] = color;
        }
    }
    Ok(d)
}
fn image_settings() -> Settings {
    Settings {
        input: Input::Image(1),
        bands: 3,
        ..Default::default()
    }
}
pub fn image_outputs() -> Result<(), String> {
    let mut d = image(false)?;
    d.layers[0].locked = true;
    let original = d.snapshot();
    let p = prep(&d, image_settings())?;
    check(
        d.snapshot() == original
            && p.added.len() == 2
            && p.visible_pixels == 1024
            && p.edge_pixels > 0,
        "Image preparation mutated original / wrong outputs",
    )?;
    let preview = p.preview(0)?;
    check(
        preview.layers[0] == d.layers[0]
            && preview.layers[1].pixels[32 * 64 + 32] == [188, 188, 188, 255]
            && preview.layers[1].pixels[0] == [0; 4],
        "Tone bands / transparent hidden RGB failed",
    )?;
    check(
        preview.layers[2].pixels[32 * 64 + 32] == [0; 4]
            && preview.layers[2].pixels[16 * 64 + 16] == [0, 0, 0, 255],
        "Uniform interior / alpha contour incorrect",
    )?;
    p.commit(&mut d)?;
    let created = d.snapshot();
    d.undo();
    check(d.snapshot() == original, "Extraction not one Undo")?;
    d.redo();
    check(d.snapshot() == created, "Extraction Redo failed")?;
    let source = crate::render::Renderer::new(&d).pixel([16.5, 16.5]);
    check(
        source == [0, 0, 0, 255],
        "Contour layer not above source / tones",
    )?;
    let mut uniform = crate::model_library_verify::blank()?;
    uniform.layers[0].pixels.fill([128, 128, 128, 255]);
    let before = uniform.snapshot();
    let p = prep(
        &uniform,
        Settings {
            input: Input::Image(1),
            tones: false,
            silhouette: false,
            ..Default::default()
        },
    )?;
    check(
        p.edge_pixels == 0 && p.added.is_empty(),
        "Uniform image generated contours",
    )?;
    check(
        !p.commit(&mut uniform)? && uniform.snapshot() == before,
        "Empty extraction changed document",
    )?;
    let counts = uniform.history_counts();
    check(
        !prep(
            &uniform,
            Settings {
                input: Input::Image(1),
                strength: 0.,
                ..Default::default()
            },
        )?
        .commit(&mut uniform)?
            && uniform.history_counts() == counts,
        "Strength0 added history",
    )?;
    let dense = image(true)?;
    let p = prep(
        &dense,
        Settings {
            input: Input::Image(1),
            silhouette: false,
            ..Default::default()
        },
    )?;
    check(
        !p.warnings.is_empty() && p.edge_pixels == p.visible_pixels,
        "Dense contour warning absent",
    )?;
    // New result layers adopt the project precision / working profile.
    let mut high = Document::create_with_color(
        crate::document::CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: crate::document::Background::Transparent,
        },
        B,
        crate::color_storage::Config::default(),
    )
    .map_err(|e| e.to_string())?;
    let extent = crate::document::PlacedRaster {
        width: 64,
        height: 64,
        offset: [0, 0],
    };
    crate::color_storage::set_samples(
        &mut high.layers[0],
        &high.color,
        extent,
        vec![crate::color_precision::Sample::from8([188, 188, 188, 255]); 4096],
    )?;
    let p = prep(
        &high,
        Settings {
            input: Input::Image(1),
            lines: false,
            ..Default::default()
        },
    )?;
    let out = p.preview(0)?;
    check(
        out.layers[1].precise.is_some()
            && out.color == high.color
            && out.layers[0] == high.layers[0],
        "High-precision result / original not preserved",
    )?;
    Ok(())
}
fn plane(d: &mut Document, position: [f64; 3], scale: [f64; 3]) -> Result<(), String> {
    lib::prepare(
        d,
        lib::Edit::Primitive {
            shape: Shape::Plane,
            detail: 6,
            placement: Placement {
                position,
                scale,
                fit: None,
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(d)?;
    Ok(())
}
pub fn scene() -> Result<Document, String> {
    let mut d = crate::model_library_verify::blank()?;
    plane(&mut d, [0.; 3], [2., 2., 1.])?;
    let mut s = d.scene.clone();
    let surface = s.world3d.surfaces.get_mut(&1).unwrap();
    surface.normals = vec![[0., 0., 1.], [0.8, 0., 0.6]];
    surface.triangles[0].n = Some([0; 3]);
    surface.triangles[1].n = Some([1; 3]);
    d.edit_scene(s)?;
    Ok(d)
}
pub fn geometry() -> Result<(), String> {
    let mut d = scene()?;
    let s = Settings {
        input: Input::Scene,
        tones: false,
        luminance: false,
        depth: false,
        normals: true,
        silhouette: false,
        normal_angle: 20.,
        ..Default::default()
    };
    let before = d.snapshot();
    let p = prep(&d, s)?;
    check(
        p.edge_pixels > 0 && p.added.len() == 1 && d.snapshot() == before,
        "Visible normal discontinuity missing / source changed",
    )?;
    plane(&mut d, [0., 0., 1.], [3., 3., 1.])?;
    let hidden = d.snapshot();
    let p = prep(&d, s)?;
    check(
        p.edge_pixels == 0 && p.added.is_empty() && d.snapshot() == hidden,
        "Hidden backface normal boundary leaked",
    )?;
    let mut depth = crate::model_library_verify::blank()?;
    plane(&mut depth, [0.; 3], [2., 2., 1.])?;
    plane(&mut depth, [0.5, 0., 1.], [0.5, 1., 1.])?;
    let p = prep(
        &depth,
        Settings {
            tones: false,
            luminance: false,
            normals: false,
            silhouette: false,
            depth: true,
            depth_threshold: 0.01,
            ..Default::default()
        },
    )?;
    check(p.edge_pixels > 0, "Visible depth boundary absent")?;
    let mut hidden_rgb = depth.scene.clone();
    hidden_rgb
        .catalog
        .world_info
        .entry(2)
        .or_default()
        .properties
        .hidden = true;
    depth.edit_scene(hidden_rgb)?;
    let p = prep(
        &depth,
        Settings {
            tones: false,
            luminance: false,
            normals: false,
            silhouette: false,
            depth: true,
            depth_threshold: 0.01,
            ..Default::default()
        },
    )?;
    check(p.edge_pixels == 0, "Hidden object depth boundary leaked")?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    image_outputs()?;
    geometry()?;
    let dir = root.join("line-tones-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = image(false)?;
        let original = d.snapshot();
        let p = prep(&d, image_settings())?;
        drop(p);
        check(d.snapshot() == original, "Cancelled output changed source")?;
        prep(&d, image_settings())?.commit(&mut d)?;
        let before = d.snapshot();
        let path = dir.join("derived.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == before,
            "Native result layers / source differ",
        )?;
        let png = dir.join("derived.png");
        crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        let pixels = image::open(png)
            .map_err(|e| e.to_string())?
            .to_rgba8()
            .into_raw();
        let r = crate::render::Renderer::new(&d);
        for y in 0..64 {
            for x in 0..64 {
                check(
                    pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4]
                        == r.pixel([x as f64 + 0.5, y as f64 + 0.5]),
                    "Extracted PNG differs",
                )?;
            }
        }
        let original = d.snapshot();
        check(
            core::prepare(&d, image_settings(), 1, &AtomicBool::new(false)).is_err()
                && core::prepare(&d, image_settings(), B, &AtomicBool::new(true)).is_err()
                && d.snapshot() == original,
            "Budget / cancel mutated source",
        )?;
        for s in [
            Settings {
                luma_threshold: f64::NAN,
                ..image_settings()
            },
            Settings {
                radius: 0,
                ..image_settings()
            },
            Settings {
                input: Input::Image(999),
                ..image_settings()
            },
        ] {
            check(prep(&d, s).is_err(), "Invalid extraction input accepted")?;
        }
        let p = prep(&d, image_settings())?;
        d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
        let changed = d.snapshot();
        check(
            p.commit(&mut d).is_err() && d.snapshot() == changed,
            "Stale layers committed",
        )?;
        let p = prep(&d, image_settings())?;
        d.current_frame = 1;
        check(!p.matches(&d), "Changed frame accepted old extraction")?;
        d.current_frame = 0;
        let mut full = image(false)?;
        full.layers = vec![full.layers[0].clone(); 256];
        check(
            prep(&full, image_settings()).is_err(),
            "Layer limit exceeded",
        )?;
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn image_bands_alpha_uniform_zero_strength_dense_and_precision() {
        super::image_outputs().unwrap();
    }
    #[test]
    fn visible_normal_depth_and_hidden_boundaries() {
        super::geometry().unwrap();
    }
    #[test]
    fn native_png_cancel_budget_stale_and_layer_limits() {
        let t = crate::test_support::TempDir::new();
        super::verify(&t.0).unwrap();
    }
}
