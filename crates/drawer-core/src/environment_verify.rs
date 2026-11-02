use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    environment::*,
    model_library::{self as lib, Edit as ModelEdit, Placement, Shape},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn fixture(root: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = root.join("panorama.png");
    let pixels = (0..32)
        .flat_map(|i| {
            if i % 8 < 4 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect::<Vec<_>>();
    use image::ImageEncoder;
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixels, 8, 4, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}
fn prepare(d: &Document, e: Edit) -> Result<Prepared, String> {
    crate::environment::prepare(d, e, B, &AtomicBool::new(false))
}
fn pixel(d: &Document) -> Result<[u8; 4], String> {
    Ok(crate::render::Renderer::new(d)
        .display_sample([32., 32.])?
        .rgba8())
}
pub fn encoding(root: &Path) -> Result<(), String> {
    use image::ImageEncoder;
    let file = root.join("gray.png");
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[128; 6], 2, 1, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    std::fs::write(&file, png).map_err(|e| e.to_string())?;
    let (data, _) = load(&file, Settings::default(), B, &AtomicBool::new(false))?;
    check(
        data.encoding == Encoding::Srgb && (data.pixels()?[0][0] - 0.2158605).abs() < 1e-6,
        "sRGB transfer was not linearized",
    )?;
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_gamma(png::ScaledFloat::new(1.));
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&[128; 6])
            .map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let (gamma, _) = load(&file, Settings::default(), B, &AtomicBool::new(false))?;
    check(
        gamma.encoding == Encoding::Gamma(1.) && (gamma.pixels()?[0][0] - 128. / 255.).abs() < 1e-6,
        "PNG gamma1 was decoded as sRGB",
    )?;
    let hdr = root.join("linear.hdr");
    let mut bytes = vec![];
    image::codecs::hdr::HdrEncoder::new(&mut bytes)
        .encode(&[image::Rgb([2., 0.5, 0.125]); 2], 2, 1)
        .map_err(|e| e.to_string())?;
    std::fs::write(&hdr, bytes).map_err(|e| e.to_string())?;
    let (hdr, _) = load(&hdr, Settings::default(), B, &AtomicBool::new(false))?;
    check(
        hdr.encoding == Encoding::LinearHdr && hdr.pixels()?[0] == [2., 0.5, 0.125],
        "HDR radiance clipped or gamma-corrected twice",
    )?;
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &vec![255; 1024 * 512 * 3],
            1024,
            512,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    let sun = root.join("bright.hdr");
    let mut bright = vec![];
    image::codecs::hdr::HdrEncoder::new(&mut bright)
        .encode(&[image::Rgb([131072.; 3]); 2], 2, 1)
        .map_err(|e| e.to_string())?;
    std::fs::write(&sun, bright).map_err(|e| e.to_string())?;
    check(
        load(&sun, Settings::default(), B, &AtomicBool::new(false))?
            .0
            .pixels()?[0]
            == [131072.; 3],
        "Bright HDR radiance was arbitrarily clipped/rejected",
    )?;
    let file = root.join("large.png");
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let (large, warnings) = load(&file, Settings::default(), B, &AtomicBool::new(false))?;
    check(
        large.width == 512 && large.height == 256 && warnings.iter().any(|w| w.contains("reduced")),
        "Large panorama not reduced/noticed",
    )?;
    Ok(())
}
pub fn rotation_and_light(root: &Path) -> Result<(), String> {
    let path = fixture(root)?;
    let mut d = crate::model_library_verify::blank()?;
    d.layers[0].pixels.fill([0, 255, 0, 255]);
    let before = d.snapshot();
    let settings = Settings {
        illumination: false,
        ..Default::default()
    };
    let p = prepare(&d, Edit::Load { path, settings })?;
    check(
        d.snapshot() == before,
        "Preparing panorama mutated document",
    )?;
    p.commit(&mut d)?;
    check(
        pixel(&d)? == [0, 0, 255, 255],
        "Panorama not above earlier2D content / camera direction differs",
    )?;
    let first = d.snapshot();
    let environment = d.scene.world3d.environment.clone();
    prepare(
        &d,
        Edit::Settings(Settings {
            yaw: 180.,
            ..settings
        }),
    )?
    .commit(&mut d)?;
    check(
        pixel(&d)? == [255, 0, 0, 255]
            && d.scene.worlds == first.scene.worlds
            && d.scene.world3d.nodes == first.scene.world3d.nodes,
        "Environment yaw changed models / background did not turn",
    )?;
    d.undo();
    check(d.snapshot() == first, "Environment settings not one Undo")?;
    d.redo();
    check(pixel(&d)? == [255, 0, 0, 255], "Environment Redo differs")?;
    d.undo();
    let mut scene = d.scene.clone();
    scene.world3d.lens.as_mut().unwrap().angles[1] = 180.;
    d.edit_scene(scene)?;
    check(
        pixel(&d)? == [255, 0, 0, 255] && d.scene.world3d.environment == environment,
        "Camera rotation changed environment metadata / panorama did not follow view",
    )?;
    let mut plane = crate::model_library_verify::blank()?;
    lib::prepare(
        &plane,
        ModelEdit::Primitive {
            shape: Shape::Plane,
            detail: 6,
            placement: Placement {
                fit: None,
                scale: [2., 2., 1.],
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut plane)?;
    let mut scene = plane.scene.clone();
    scene.world3d.surfaces.get_mut(&1).unwrap().materials[0].rgba = [1.; 4];
    plane.edit_scene(scene)?;
    let data = Data::from_pixels(
        "constant".into(),
        Encoding::Srgb,
        2,
        1,
        &[[0.2158605; 3]; 2],
        Settings {
            background: false,
            ..Default::default()
        },
    )?;
    let mut scene = plane.scene.clone();
    scene.world3d.environment = Some(data);
    plane.edit_scene(scene)?;
    prepare(
        &plane,
        Edit::Settings(Settings {
            background: false,
            ..Default::default()
        }),
    )?
    .commit(&mut plane)?;
    let color = pixel(&plane)?;
    check(
        color[..3].iter().all(|v| v.abs_diff(128) <= 1) && color[3] == 255,
        "Constant diffuse image light/gamma incorrect",
    )?;
    let original_geometry = plane.scene.worlds.clone();
    let env = plane.scene.world3d.environment.clone();
    prepare(
        &plane,
        Edit::Settings(Settings {
            background: false,
            strength: 0.,
            ..Default::default()
        }),
    )?
    .commit(&mut plane)?;
    check(
        pixel(&plane)? == [0, 0, 0, 255],
        "Zero image-light strength left illumination",
    )?;
    let counts = plane.history_counts();
    prepare(
        &plane,
        Edit::Settings(Settings {
            background: false,
            strength: 0.,
            ..Default::default()
        }),
    )?
    .commit(&mut plane)?;
    check(
        plane.history_counts() == counts && plane.scene.worlds == original_geometry,
        "Identical settings added history/changed models",
    )?;
    let kept = plane.scene.world3d.environment.clone();
    crate::world3d::prepare(
        &plane,
        crate::world3d::Edit::Transform {
            ids: vec![1],
            options: crate::world3d::Preferences {
                mode: crate::world3d::Mode::Rotate,
                ..Default::default()
            },
            value: [0., 45., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut plane)?;
    check(
        plane.scene.world3d.environment == kept && plane.scene.worlds != original_geometry,
        "Model rotation changed environment / did not move model",
    )?;
    let sample = Sampler::new(env.as_ref().unwrap())?;
    let light = sample.light();
    for n in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
        check(
            light
                .diffuse(n)
                .iter()
                .all(|v| (*v - 0.2158605).abs() < 0.001),
            "Constant SH irradiance varies by direction",
        )?;
    }
    let (color, _) = load(
        &root.join("panorama.png"),
        Settings::default(),
        B,
        &AtomicBool::new(false),
    )?;
    let a = Sampler::new(&color)?.light().diffuse([1., 0., 0.]);
    let mut turned = color;
    turned.settings.yaw = 180.;
    let b = Sampler::new(&turned)?.light().diffuse([1., 0., 0.]);
    check(
        (a[0] - b[0]).abs() > 0.1 && (a[2] - b[2]).abs() > 0.1,
        "Environment light did not rotate independently",
    )?;
    Ok(())
}
pub fn native_and_precision(root: &Path) -> Result<(), String> {
    let path = fixture(root)?;
    let original = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut d = crate::model_library_verify::blank()?;
    let before = d.snapshot();
    prepare(
        &d,
        Edit::Load {
            path: path.clone(),
            settings: Settings::default(),
        },
    )?
    .commit(&mut d)?;
    let state = d.snapshot();
    d.undo();
    check(d.snapshot() == before, "Load not one Undo")?;
    d.redo();
    check(d.snapshot() == state, "Load Redo differs")?;
    let native = root.join("panorama.drawer");
    crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
    check(
        std::fs::read(&path).map_err(|e| e.to_string())? == original,
        "Source image was modified",
    )?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == state && pixel(&reopened)? == pixel(&d)?,
        "Embedded panorama depends on source file",
    )?;
    let mut bytes = std::fs::read(&native).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&113u32.to_le_bytes());
    let old = root.join("old.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, B).is_err(),
        "Native113 accepted environment map",
    )?;
    let png = root.join("panorama-output.png");
    crate::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
    let out = image::open(png).map_err(|e| e.to_string())?.to_rgba8();
    let renderer = crate::render::Renderer::new(&d);
    for y in 0..64 {
        for x in 0..64 {
            check(
                out.get_pixel(x, y).0 == renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]),
                "PNG panorama pixels differ",
            )?;
        }
    }
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
    let mut scene = high.scene.clone();
    scene.camera = d.scene.camera.clone();
    scene.world3d = d.scene.world3d.clone();
    high.edit_scene(scene)?;
    check(
        pixel(&high)? == pixel(&d)?,
        "Floating/profiled background differs from legacy renderer",
    )?;
    let float = crate::float_render::Renderer::new(&high)?;
    check(
        float.display_sample([32., 32.])?.rgba8() == pixel(&d)?,
        "Direct floating export path excludes panorama",
    )?;
    let mut legacy = d.snapshot();
    legacy.layers[0].pixels.fill([0, 255, 0, 255]);
    legacy.scene.layer_ids.initialize(1)?;
    legacy.scene.render_order.entries = vec![crate::render_order::Entry {
        key: crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(1)),
        group: crate::render_order::Group::World,
    }];
    let legacy = crate::layer_merge::preview(&legacy, 0)?;
    check(
        pixel(&legacy)? == [0, 255, 0, 255],
        "Deferred World raster did not remain above background",
    )?;
    let mut scene = high.scene.clone();
    scene.world3d.environment = Some(Data::from_pixels(
        "gray".into(),
        Encoding::Srgb,
        2,
        1,
        &[[0.2158605; 3]; 2],
        Settings {
            illumination: false,
            ..Default::default()
        },
    )?);
    high.edit_scene(scene)?;
    check(
        pixel(&high)?.iter().take(3).all(|v| v.abs_diff(128) <= 1),
        "Profiled gray background gamma differs",
    )?;
    let working = crate::color_profile::WorkingSpace::new(&high.color.profile)?;
    crate::color_storage::set_samples(
        &mut high.layers[0],
        &high.color,
        crate::document::PlacedRaster {
            width: 64,
            height: 64,
            offset: [0, 0],
        },
        vec![working.from_srgb(crate::color_precision::Sample::from8([0, 255, 0, 255]))?; 4096],
    )?;
    let mut scene = high.scene.clone();
    scene.layer_ids.initialize(1)?;
    scene.render_order.entries = vec![crate::render_order::Entry {
        key: crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(1)),
        group: crate::render_order::Group::World,
    }];
    high.edit_scene(scene)?;
    check(
        pixel(&high)? == [0, 255, 0, 255],
        "Floating ordered background covered World raster",
    )?;
    let mut hdr = crate::model_library_verify::blank()?;
    let mut scene = hdr.scene.clone();
    let (c, l) = crate::world3d::default_camera(&hdr);
    scene.camera = Some(c);
    scene.world3d.lens = Some(l);
    scene.world3d.environment = Some(Data::from_pixels(
        "hdr".into(),
        Encoding::LinearHdr,
        2,
        1,
        &[[2., 0.5, 0.125]; 2],
        Settings {
            illumination: false,
            ..Default::default()
        },
    )?);
    hdr.edit_scene(scene)?;
    let file = root.join("hdr.drawer");
    crate::files::save(&mut hdr, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, B)
            .map_err(|e| e.to_string())?
            .scene
            .world3d
            .environment
            .unwrap()
            .pixels()?[0]
            == [2., 0.5, 0.125],
        "Native clipped HDR radiance",
    )?;
    prepare(&d, Edit::Remove)?.commit(&mut d)?;
    check(
        d.scene.world3d.environment.is_none(),
        "Removal kept embedded image",
    )?;
    d.undo();
    check(
        d.scene.world3d.environment == state.scene.world3d.environment,
        "Removal Undo lost image",
    )?;
    let mut empty = crate::model_library_verify::blank()?;
    let counts = empty.history_counts();
    let state = empty.snapshot();
    check(
        !prepare(&empty, Edit::Remove)?.commit(&mut empty)?
            && empty.history_counts() == counts
            && empty.snapshot() == state,
        "Empty removal added history",
    )?;
    Ok(())
}
pub fn rejection(root: &Path) -> Result<(), String> {
    let path = fixture(root)?;
    let mut d = crate::model_library_verify::blank()?;
    let original = d.snapshot();
    for (budget, cancel) in [(1, false), (B, true)] {
        check(
            crate::environment::prepare(
                &d,
                Edit::Load {
                    path: path.clone(),
                    settings: Settings::default(),
                },
                budget,
                &AtomicBool::new(cancel),
            )
            .is_err()
                && d.snapshot() == original,
            "Budget/cancel mutated document",
        )?;
    }
    check(
        prepare(
            &d,
            Edit::Load {
                path: root.join("missing.png"),
                settings: Settings::default(),
            },
        )
        .is_err()
            && prepare(&d, Edit::Settings(Settings::default())).is_err(),
        "Missing image/embedded input accepted",
    )?;
    check(
        prepare(
            &d,
            Edit::Load {
                path: path.clone(),
                settings: Settings {
                    strength: f64::NAN,
                    ..Default::default()
                },
            },
        )
        .is_err(),
        "Invalid settings accepted",
    )?;
    let p = prepare(
        &d,
        Edit::Load {
            path: path.clone(),
            settings: Settings::default(),
        },
    )?;
    d.current_frame = 1;
    check(
        !p.matches(&d) && p.commit(&mut d).is_err(),
        "Stale frame accepted",
    )?;
    d.current_frame = 0;
    let p = prepare(
        &d,
        Edit::Load {
            path: path.clone(),
            settings: Settings::default(),
        },
    )?;
    d.edit_pixel(0, 0, 0, [1, 2, 3, 255]);
    check(p.commit(&mut d).is_err(), "Stale document accepted")?;
    check(
        Data::from_pixels(
            "bad".into(),
            Encoding::LinearHdr,
            2,
            1,
            &[[f32::NAN; 3]; 2],
            Settings::default(),
        )
        .is_err(),
        "NaN radiance accepted",
    )?;
    let mismatch = root.join("mismatch.hdr");
    std::fs::copy(&path, &mismatch).map_err(|e| e.to_string())?;
    check(
        load(&mismatch, Settings::default(), B, &AtomicBool::new(false)).is_err(),
        "Image extension/header mismatch accepted",
    )?;
    use image::ImageEncoder;
    for (name, width, height, pixels) in [
        ("square.png", 2, 2, vec![255; 16]),
        ("alpha.png", 2, 1, vec![0; 8]),
    ] {
        let mut bytes = vec![];
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        let p = root.join(name);
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
        check(
            load(&p, Settings::default(), B, &AtomicBool::new(false)).is_err(),
            "Invalid panorama aspect/alpha accepted",
        )?;
    }
    Ok(())
}
pub fn capacity(root: &Path) -> Result<(), String> {
    let mut d = crate::world_views_verify::fixture()?;
    let mut scene = d.scene.clone();
    let s = scene.world3d.surfaces.get_mut(&1).unwrap();
    s.triangles = vec![s.triangles[0].clone(); 40000];
    d.edit_scene(scene)?;
    check(
        crate::files::native_scene(&d).is_ok(),
        "Capacity fixture already exceeds native limit",
    )?;
    let original = d.snapshot();
    use image::ImageEncoder;
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &vec![255; 512 * 256 * 3],
            512,
            256,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    let file = root.join("capacity.png");
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    check(
        prepare(
            &d,
            Edit::Load {
                path: file,
                settings: Settings::default(),
            },
        )
        .is_err()
            && d.snapshot() == original,
        "Native capacity overflow was committed",
    )?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("environment-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        for (name, run) in [
            ("encoding", encoding as fn(&Path) -> Result<(), String>),
            ("light", rotation_and_light),
            ("native", native_and_precision),
            ("reject", rejection),
            ("capacity", capacity),
        ] {
            let p = dir.join(name);
            std::fs::create_dir(&p).map_err(|e| e.to_string())?;
            run(&p)?;
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn ldr_gamma_hdr_and_reduction() {
        let t = crate::test_support::TempDir::new();
        super::encoding(&t.0).unwrap();
    }
    #[test]
    fn diffuse_yaw_camera_model_independence_and_zero() {
        let t = crate::test_support::TempDir::new();
        super::rotation_and_light(&t.0).unwrap();
    }
    #[test]
    fn native114_source_independence_png_float_and_undo() {
        let t = crate::test_support::TempDir::new();
        super::native_and_precision(&t.0).unwrap();
    }
    #[test]
    fn missing_invalid_cancel_budget_stale_and_aspect() {
        let t = crate::test_support::TempDir::new();
        super::rejection(&t.0).unwrap();
    }
    #[test]
    fn native_capacity_before_commit() {
        let t = crate::test_support::TempDir::new();
        super::capacity(&t.0).unwrap();
    }
}
