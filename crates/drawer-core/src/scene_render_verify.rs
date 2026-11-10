use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    lighting::{self, Kind, Light, Style},
    scene_render::*,
    world_math::dot,
    world3d::{self, Matrix},
};
use std::{path::Path, sync::atomic::AtomicBool};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
fn apply(d: &mut Document, s: lighting::Settings) -> Result<(), String> {
    lighting::prepare(d, s, B, &AtomicBool::new(false))?.commit(d)?;
    Ok(())
}
fn pixel(d: &Document) -> [u8; 4] {
    crate::render::Renderer::new(d).pixel([32., 32.])
}
fn settings() -> lighting::Settings {
    lighting::Settings {
        style: Style::Flat,
        ambient: 0.,
        render: Some(Default::default()),
        lights: vec![Light {
            kind: Kind::Directional {
                direction: [-1., 0., -1.],
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}
pub fn shadows() -> Result<(), String> {
    let mut d = crate::lighting_verify::fixture()?;
    let mut s = settings();
    for cast in [false, true] {
        for receive in [false, true] {
            let r = s.render.as_mut().unwrap();
            r.objects.insert(
                1,
                Object {
                    receive,
                    ..Default::default()
                },
            );
            r.objects.insert(
                2,
                Object {
                    cast,
                    ..Default::default()
                },
            );
            apply(&mut d, s.clone())?;
            let color = pixel(&d);
            check(
                if cast && receive {
                    color == [0, 0, 0, 255]
                } else {
                    color[0] > 200
                },
                "Shadow cast/receive four-way result incorrect",
            )?;
        }
    }
    for target in [
        ShadowTarget::Off,
        ShadowTarget::Models,
        ShadowTarget::Ground,
        ShadowTarget::Both,
    ] {
        for ground in [false, true] {
            let r = s.render.as_mut().unwrap();
            r.target = target;
            r.objects.get_mut(&1).unwrap().ground = ground;
            let expected = matches!(target, ShadowTarget::Both)
                || target == ShadowTarget::Models && !ground
                || target == ShadowTarget::Ground && ground;
            apply(&mut d, s.clone())?;
            check(
                (pixel(&d)[0] == 0) == expected,
                "Ground/model shadow target incorrect",
            )?;
        }
    }
    for quality in [
        ShadowQuality::Low,
        ShadowQuality::Medium,
        ShadowQuality::High,
    ] {
        s.render.as_mut().unwrap().shadow = quality;
        apply(&mut d, s.clone())?;
        check(d.scene.world3d.valid(&d.scene), "Shadow resolution invalid")?;
    }
    let sample = [0.0123, 0.0456, 0.0789];
    let a = shadow_sample(sample, [0., 0., 1.], 4., 256);
    let b = shadow_sample(sample, [0., 0., 1.], 4., 1024);
    check(
        a != b && a[2] == sample[2] && b[2] == sample[2],
        "Shadow grid quality did not change texel sample",
    )?;
    s.lights[0].kind = Kind::Directional {
        direction: [-0.37, -0.13, -1.],
    };
    s.render.as_mut().unwrap().target = ShadowTarget::Both;
    s.render.as_mut().unwrap().shadow = ShadowQuality::Low;
    apply(&mut d, s.clone())?;
    let low = crate::world_render::Trace::new(&d).unwrap();
    s.render.as_mut().unwrap().shadow = ShadowQuality::High;
    apply(&mut d, s.clone())?;
    let high = crate::world_render::Trace::new(&d).unwrap();
    check(
        (0..1000).any(|i| {
            let p = [31. + i as f64 * 0.01, 32.13];
            low.pixel(0, p) != high.pixel(0, p)
        }),
        "Shadow resolutions produced identical boundary samples",
    )?;
    s.shadows = false;
    s.lights[0].kind = Kind::Directional {
        direction: [0., 0., -1.],
    };
    apply(&mut d, s.clone())?;
    let mut scene = d.scene.clone();
    scene.world3d.lens.as_mut().unwrap().angles = [0., 15., 0.];
    d.edit_scene(scene)?;
    let fixed = pixel(&d);
    s.lights[0].follow_camera = true;
    apply(&mut d, s)?;
    check(
        fixed[0] == 255 && pixel(&d)[0] < 255 && pixel(&d)[3] == 255,
        "Camera-relative light indistinguishable from fixed light",
    )?;
    Ok(())
}
pub fn fogs() -> Result<(), String> {
    let d = crate::lighting_verify::fixture()?;
    let (mut c, l) = world3d::default_camera(&d);
    c.position = [0.; 3];
    let mut f = Fog {
        start: 0.,
        end: 10.,
        density: 1.,
        ..Default::default()
    };
    let a = [0., 0., -5.];
    let b = [3., 0., -5.];
    check(
        f.coverage(a, &c, l) == f.coverage(b, &c, l),
        "Depth fog used Euclidean distance",
    )?;
    f.kind = FogKind::Distance;
    check(
        f.coverage(b, &c, l) > f.coverage(a, &c, l),
        "Distance fog ignored lateral distance",
    )?;
    f.kind = FogKind::HeightAbove;
    let low = f.coverage([0., 2., -5.], &c, l);
    let high = f.coverage([0., 8., -5.], &c, l);
    check(high > low, "Height-above fog reversed")?;
    f.kind = FogKind::HeightBelow;
    check(
        (f.coverage([0., 2., -5.], &c, l) - high).abs() < 1e-12,
        "Height-below fog not inverted",
    )?;
    f.density = 0.;
    check(f.coverage(a, &c, l) == 0., "Density0 fog not identity")?;
    f.end = f.start;
    check(!f.valid(), "Fog start=end accepted")?;
    let mut d = crate::lighting_verify::fixture()?;
    let mut s = settings();
    s.style = Style::Unlit;
    apply(&mut d, s.clone())?;
    let before = pixel(&d);
    s.render.as_mut().unwrap().fog = Some(Fog {
        kind: FogKind::Depth,
        start: 0.,
        end: 10.,
        density: 2.,
        color: [1., 0., 0.],
    });
    apply(&mut d, s.clone())?;
    check(
        pixel(&d)[0] == 255 && pixel(&d)[1] < before[1] && pixel(&d)[3] == 255,
        "Actual shaded fog color/alpha wrong",
    )?;
    let snap = d.snapshot();
    s.render.as_mut().unwrap().fog.as_mut().unwrap().end = 0.;
    check(
        lighting::prepare(&d, s, B, &AtomicBool::new(false)).is_err() && d.snapshot() == snap,
        "Invalid fog changed source",
    )?;
    Ok(())
}
pub fn alpha_depth() -> Result<(), String> {
    let mut d = crate::model_library_verify::blank()?;
    for (position, scale) in [
        ([0., -0.5, 0.5], [1.; 3]),
        ([0., -0.5, 0.], [1.; 3]),
        ([0., -1., 0.], [8., 0.02, 8.]),
    ] {
        crate::model_library::prepare(
            &d,
            crate::model_library::Edit::Primitive {
                shape: crate::model_library::Shape::Box,
                detail: 6,
                placement: crate::model_library::Placement {
                    position,
                    scale,
                    fit: None,
                    ..Default::default()
                },
            },
            B,
            &AtomicBool::new(false),
        )?
        .commit(&mut d)?;
    }
    let mut scene = d.scene.clone();
    for (id, color) in [
        (1, [1., 0., 0., 0.5]),
        (2, [0., 0., 1., 0.5]),
        (3, [0.7, 0.7, 0.7, 1.]),
    ] {
        scene.world3d.surfaces.get_mut(&id).unwrap().materials[0].rgba = color;
    }
    scene.camera.as_mut().unwrap().position = [0., 2., 6.];
    scene.world3d.lens.as_mut().unwrap().angles = [-20., 0., 0.];
    d.edit_scene(scene)?;
    let mut s = settings();
    s.style = Style::Unlit;
    s.shadows = false;
    let r = s.render.as_mut().unwrap();
    for id in [1, 2] {
        r.objects.insert(
            id,
            Object {
                alpha: Alpha::Blend,
                ..Default::default()
            },
        );
    }
    r.objects.insert(
        3,
        Object {
            ground: true,
            ..Default::default()
        },
    );
    apply(&mut d, s.clone())?;
    let baseline = pixel(&d);
    let trace = crate::world_render::Trace::new(&d).ok_or("No trace")?;
    let fragments = trace.fragments(&[0, 1, 2], [32., 32.]);
    check(
        fragments.iter().filter(|(i, _)| *i < 2).count() >= 4,
        "Translucent solids lost back faces",
    )?;
    check(
        fragments.iter().any(|(i, _)| *i == 2),
        "Ground surface missing behind translucent solids",
    )?;
    check(
        trace.fragments_cached(&[0, 1, 2], [32., 32.]) == fragments
            && trace.fragments_cached(&[0, 1, 2], [32., 32.]) == fragments,
        "Repeated reduced-fragment cache changed output",
    )?;
    check(
        trace.fragments_cached(&[0], [32., 32.]) == trace.fragments(&[0], [32., 32.]),
        "Fragment cache leaked another model group",
    )?;
    let expected = fragments
        .iter()
        .fold([0; 4], |a, (_, b)| crate::render::over(a, *b));
    check(
        baseline == expected,
        "Scene did not compose all translucent surfaces in depth order",
    )?;
    let mut scene = d.scene.clone();
    scene.render_order.entries = crate::render_order::sources(&scene);
    scene.render_order.entries.reverse();
    d.edit_scene(scene)?;
    check(
        pixel(&d) == baseline,
        "Translucency depends on model layer order",
    )?;
    d.color = crate::color_storage::Config::default();
    let floating = crate::render::Renderer::new(&d)
        .display_sample([32., 32.])?
        .rgba8();
    check(
        floating
            .into_iter()
            .zip(baseline)
            .all(|(a, b)| (a as i32 - b as i32).abs() <= 2),
        "Floating/profile transparency differs from legacy depth composition",
    )?;
    let export = crate::render::Renderer::new(&d).output(d.dpi, B / 8)?.2;
    let mut scene = d.scene.clone();
    scene
        .world3d
        .lighting
        .as_mut()
        .unwrap()
        .render
        .as_mut()
        .unwrap()
        .quality = EditQuality::HighSpeed;
    d.edit_scene(scene)?;
    check(
        crate::render::Renderer::new(&d).output(d.dpi, B / 8)?.2 == export
            && viewport_reduction(&d) == 4,
        "Editing quality altered final output",
    )?;
    let mut d = crate::lighting_verify::fixture()?;
    let mut scene = d.scene.clone();
    scene.world3d.surfaces.get_mut(&1).unwrap().materials[0].rgba[3] = 0.5;
    d.edit_scene(scene)?;
    let mut s = settings();
    s.style = Style::Unlit;
    s.render.as_mut().unwrap().objects.insert(
        1,
        Object {
            alpha: Alpha::Threshold,
            threshold: 0.5,
            ..Default::default()
        },
    );
    apply(&mut d, s.clone())?;
    check(pixel(&d)[3] == 255, "Threshold equality removed pixel")?;
    s.render
        .as_mut()
        .unwrap()
        .objects
        .get_mut(&1)
        .unwrap()
        .threshold = 0.5001;
    apply(&mut d, s.clone())?;
    check(pixel(&d)[3] == 0, "Threshold below cutoff survived")?;
    s.render
        .as_mut()
        .unwrap()
        .objects
        .get_mut(&1)
        .unwrap()
        .alpha = Alpha::Opaque;
    apply(&mut d, s.clone())?;
    check(pixel(&d)[3] == 255, "Opaque override left source alpha")?;
    s.render
        .as_mut()
        .unwrap()
        .objects
        .get_mut(&1)
        .unwrap()
        .alpha = Alpha::Blend;
    s.render
        .as_mut()
        .unwrap()
        .objects
        .get_mut(&1)
        .unwrap()
        .alpha_scale = 0.5;
    apply(&mut d, s)?;
    check(
        (pixel(&d)[3] as i32 - 64).abs() <= 1,
        "Blend alpha multiplier ignored",
    )?;
    let mut scene = d.scene.clone();
    scene.world3d.surfaces.get_mut(&1).unwrap().materials[0].rgba[3] = 0.;
    d.edit_scene(scene)?;
    let mut s = d.scene.world3d.lighting.clone().unwrap();
    let o = s.render.as_mut().unwrap().objects.get_mut(&1).unwrap();
    o.alpha = Alpha::Threshold;
    o.threshold = 0.;
    apply(&mut d, s.clone())?;
    check(
        pixel(&d)[3] == 255,
        "Threshold0 failed to include alpha0 equality",
    )?;
    s.render
        .as_mut()
        .unwrap()
        .objects
        .get_mut(&1)
        .unwrap()
        .threshold = 1.;
    apply(&mut d, s)?;
    check(pixel(&d)[3] == 0, "Threshold1 included alpha0")?;
    Ok(())
}
pub fn billboards() -> Result<(), String> {
    let d = crate::lighting_verify::fixture()?;
    let (mut c, mut l) = world3d::default_camera(&d);
    l.angles = [25., 35., 10.];
    c.position = [4., 3., 6.];
    let frame = Matrix::scale([-2., 1., 0.7])
        .then(Matrix::euler([5., 80., 35.]))
        .then(Matrix::translation([1., 0., 0.]));
    for mode in [Billboard::AllAxes, Billboard::OnlyHorizontal] {
        let b = billboard(frame, mode, &c, l);
        check(
            b.valid()
                && b.origin() == frame.origin()
                && (b.determinant() - frame.determinant()).abs() < 1e-8,
            "Billboard lost position/scale/reflection",
        )?;
        if mode == Billboard::OnlyHorizontal {
            check(b.axis(1)[1] > 0.999, "Horizontal billboard tilted world up")?;
        } else {
            check(
                dot(b.axis(2), Matrix::euler(l.angles).axis(2)).abs() > 0.999,
                "All-axis billboard not camera facing",
            )?;
        }
    }
    let mut d = crate::lighting_verify::fixture()?;
    let original = d.scene.worlds.clone();
    let mut s = settings();
    s.style = Style::Unlit;
    s.render.as_mut().unwrap().outline = 2;
    s.render.as_mut().unwrap().line = [0, 255, 0, 255];
    apply(&mut d, s.clone())?;
    let image = crate::render::Renderer::new(&d).output(d.dpi, B / 8)?.2;
    check(
        image.contains(&[0, 255, 0, 255]),
        "Outline width/color produced no outline",
    )?;
    s.render.as_mut().unwrap().objects.insert(
        2,
        Object {
            billboard: Billboard::AllAxes,
            ..Default::default()
        },
    );
    apply(&mut d, s)?;
    check(
        d.scene.worlds == original,
        "Billboard or outline baked geometry",
    )?;
    let p = [1.2, 2.4];
    check(edit_point(&d, p) == p, "Normal edit mode reduced points")?;
    let mut scene = d.scene.clone();
    scene
        .world3d
        .lighting
        .as_mut()
        .unwrap()
        .render
        .as_mut()
        .unwrap()
        .quality = EditQuality::Fast;
    d.edit_scene(scene)?;
    check(
        edit_point(&d, p) != p && viewport_reduction(&d) == 1,
        "Fast quality reduced other layers",
    )?;
    let mut scene = d.scene.clone();
    scene.world3d.lens.as_mut().unwrap().shift = [0.01, 0.01];
    d.edit_scene(scene)?;
    let normal = crate::render::Renderer::new(&d);
    let editing = crate::render::Renderer::editing_preview(&d, None);
    check(
        (0..64)
            .flat_map(|y| (0..64).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
            .any(|p| normal.pixel(p) != editing.pixel(p)),
        "Fast edit quality did not reduce actual displayed 3D detail",
    )?;
    let sample = crate::render::Renderer::editing_preview(&d, None);
    check(sample.error().is_none(), "Editing quality renderer failed")?;
    let mut scene = d.scene.clone();
    let c = scene.camera.as_mut().unwrap();
    c.position = [0., 0., 2.];
    c.page_bounds = crate::scene::Rect {
        min: [16., 16.],
        max: [48., 48.],
    };
    let options = scene
        .world3d
        .lighting
        .as_mut()
        .unwrap()
        .render
        .as_mut()
        .unwrap();
    options.outline = 0;
    options.quality = EditQuality::Normal;
    scene.comics.masks.push(crate::comics::Mask {
        target: crate::layer_search::Key::World(1),
        polygon: vec![[0., 0.], [64., 0.], [64., 64.], [0., 64.]],
        space: Default::default(),
    });
    d.edit_scene(scene)?;
    check(
        crate::render::Renderer::new(&d).pixel([12.5, 32.5])[3] > 0,
        "Advanced3D clipped a comic-owned model outside the camera rectangle",
    )?;
    Ok(())
}
pub fn persistence(root: &Path) -> Result<(), String> {
    let mut d = crate::lighting_verify::fixture()?;
    let mut s = settings();
    s.lights[0].follow_camera = true;
    let r = s.render.as_mut().unwrap();
    r.tint = [0.5, 0.7, 1.];
    r.fog = Some(Fog {
        kind: FogKind::HeightBelow,
        start: -2.,
        end: 3.,
        density: 4.,
        color: [0.2, 0.3, 0.4],
    });
    r.outline = 3;
    r.line = [20, 40, 60, 128];
    r.quality = EditQuality::Fast;
    r.shadow = ShadowQuality::Low;
    r.target = ShadowTarget::Ground;
    r.objects.insert(
        1,
        Object {
            cast: false,
            ground: true,
            receive: true,
            billboard: Billboard::OnlyHorizontal,
            alpha: Alpha::Blend,
            threshold: 0.2,
            alpha_scale: 0.7,
        },
    );
    apply(&mut d, s.clone())?;
    let snapshot = d.snapshot();
    let pixels = crate::render::Renderer::new(&d).output(d.dpi, B / 8)?.2;
    let path = root.join("render.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, B).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == snapshot
            && crate::render::Renderer::new(&reopened)
                .output(d.dpi, B / 8)?
                .2
                == pixels,
        "Native119 options/pixels differ",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    crate::save_generation::verify::legacy_version(&mut bytes, 118);
    std::fs::write(root.join("old-render.drawer"), bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&root.join("old-render.drawer"), B).is_err(),
        "Native118 accepted advanced render options",
    )?;
    let stale = lighting::prepare(&d, s.clone(), B, &AtomicBool::new(false))?;
    d.current_frame += 1;
    check(
        stale.commit(&mut d).is_err(),
        "Lighting accepted a stale frame",
    )?;
    d.current_frame -= 1;
    for (budget, cancel) in [(1, false), (B, true)] {
        check(
            lighting::prepare(&d, s.clone(), budget, &AtomicBool::new(cancel)).is_err(),
            "Lighting budget/cancel bypass",
        )?;
    }
    crate::model_library::prepare(
        &d,
        crate::model_library::Edit::Duplicate {
            ids: vec![1],
            offset: [2., 0., 0.],
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    let options = d
        .scene
        .world3d
        .lighting
        .as_ref()
        .unwrap()
        .render
        .as_ref()
        .unwrap();
    check(
        options.objects.get(&3) == options.objects.get(&1),
        "Duplicate lost independent object render policy",
    )?;
    d.undo();
    check(
        d.snapshot() == snapshot,
        "Duplicate Undo did not restore render metadata",
    )?;
    for source in [
        LightSource::Files,
        LightSource::Material,
        LightSource::Panorama,
    ] {
        s.render.as_mut().unwrap().source = source;
        check(
            lighting::prepare(&d, s.clone(), B, &AtomicBool::new(false)).is_err(),
            "Missing environment snapshot silently accepted",
        )?;
    }
    let file = crate::environment_verify::fixture(&root.join("local-environment"))?;
    crate::environment::prepare(
        &d,
        crate::environment::Edit::Load {
            path: file,
            settings: crate::environment::Settings {
                illumination: true,
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut d)?;
    for source in [
        LightSource::Files,
        LightSource::Material,
        LightSource::Panorama,
    ] {
        s.render.as_mut().unwrap().source = source;
        apply(&mut d, s.clone())?;
        check(
            d.scene
                .world3d
                .environment
                .as_ref()
                .unwrap()
                .settings
                .illumination,
            "Environment source disabled snapshot",
        )?;
    }
    let base = d.snapshot();
    s.render.as_mut().unwrap().source = LightSource::SingleColor;
    apply(&mut d, s)?;
    d.undo();
    check(d.snapshot() == base, "Render change not one Undo")?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("scene-render-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        shadows()?;
        fogs()?;
        alpha_depth()?;
        billboards()?;
        persistence(&dir)
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
#[cfg(test)]
mod tests {
    #[test]
    fn camera_follow_cast_receive_ground_targets_and_shadow_grid() {
        super::shadows().unwrap();
    }
    #[test]
    fn fog_depth_distance_height_direction_density_and_bounds() {
        super::fogs().unwrap();
    }
    #[test]
    fn two_translucent_solids_ground_depth_order_alpha_boundaries_and_final_quality() {
        super::alpha_depth().unwrap();
    }
    #[test]
    fn billboard_axes_reflection_outline_and_editing_quality() {
        super::billboards().unwrap();
    }
    #[test]
    fn native119_options_pixels_sources_undo_budget_cancel_and_stale() {
        let t = crate::test_support::TempDir::new();
        super::persistence(&t.0).unwrap();
    }
}
