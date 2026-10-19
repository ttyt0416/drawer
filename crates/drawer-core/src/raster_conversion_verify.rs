use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    layer_properties::{self as p, Target},
    layer_search::{Expression, Key},
    raster_conversion::{self as core, Action},
    render::Renderer,
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn mixed() -> Result<crate::document::Document, String> {
    let mut d = crate::line_tools_verify::fixture()?;
    d.layers[0].pixels.fill([70, 90, 110, 128]);
    d.scene.objects.push(crate::scene::PageObject::Line {
        start: [0., 12.],
        end: [60., 12.],
        width_px: 5.,
        color: [200, 40, 80, 128],
    });
    d.scene.objects.push(crate::scene::PageObject::Text {
        baseline: [25., 36.],
        text: "Hi".into(),
        size_px: 10.,
        color: [160, 60, 100, 128],
    });
    let v = d.scene.lines.layers[0].id;
    d.scene.layer_properties.vectors.insert(
        v,
        p::Properties {
            opacity: 65,
            blend: crate::layer_blend::Mode::Multiply,
            ..Default::default()
        },
    );
    let top = d.scene.lines.add_layer("Upper vector")?;
    d.scene.lines.add_path(
        top,
        vec![[8., 12.], [50., 12.]],
        false,
        3.,
        [10, 190, 40, 128],
    )?;
    d.scene.camera = Some(crate::scene::Camera {
        page_bounds: crate::scene::Rect {
            min: [0.; 2],
            max: [64., 48.],
        },
        position: [0.; 3],
        pixels_per_unit: 1.,
    });
    d.scene.worlds.push(crate::scene::WorldMesh {
        vertices: vec![[-10., 0., 0.], [10., 0., 0.]],
        edges: vec![[0, 1]],
        color: [60, 100, 180, 128],
    });
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = mixed()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    let obj = core::prepare(&d, Key::Object(1), Action::Rasterize(Expression::Color))?;
    let obj_key = obj.selected;
    check(
        obj.maximum_delta <= 1 && d.snapshot() == original,
        "Object raster preflight/comparison changed source",
    )?;
    d.commit_state(obj.state)?;
    let vector = d.scene.lines.layers[0].id;
    let v = core::prepare(
        &d,
        Key::Layer(Target::Vector(vector)),
        Action::Rasterize(Expression::Color),
    )?;
    let selected = v.selected;
    check(v.maximum_delta <= 1, "Vector raster compositing mismatch")?;
    d.commit_state(v.state)?;
    let text = core::prepare(&d, Key::Object(2), Action::Rasterize(Expression::Color))?;
    check(
        text.maximum_delta <= 1,
        "Text glyph rasterization changed appearance",
    )?;
    d.commit_state(text.state)?;
    let Key::Layer(Target::Raster(id)) = selected else {
        return Err("Raster output is not paintable storage".into());
    };
    let n = crate::layers::index(&d, id).ok_or("New raster missing")?;
    check(
        p::paintable(&d, n).is_ok()
            && d.scene.objects.is_empty()
            && d.scene.lines.layers.len() == 1
            && d.history_counts().0 == counts.0 + 3,
        "Rasterization source removal/one Undo per action",
    )?;
    let visible = crate::layer_pick::visible(
        &d,
        crate::layer_pick::Settings {
            include_locked: true,
            ..Default::default()
        },
    );
    let keys = visible
        .iter()
        .filter_map(|t| crate::layer_catalog::from_thumbnail(&d, *t))
        .collect::<Vec<_>>();
    check(
        keys.iter().position(|k| *k == selected) < keys.iter().position(|k| *k == obj_key),
        "Rasterized canvas pick order",
    )?;
    let rows = crate::layer_catalog::rows(&d)?;
    check(
        rows.iter().position(|r| r.key == obj_key) < rows.iter().position(|r| r.key == selected),
        "Rasterized layer list order",
    )?;
    let after = d.snapshot();
    d.view.angle = 1.2;
    let path = root.join("raster-order.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == after,
        "Raster order/placement native roundtrip",
    )?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut invalid = crate::layer_merge::preview(&d.snapshot(), 0)?;
    invalid.scene.render_order.entries[0].key = Key::Layer(Target::Raster(999999));
    check(
        crate::files::save(&mut invalid, &path).is_err()
            && std::fs::read(&path).map_err(|e| e.to_string())? == bytes,
        "Invalid raster source identity damaged native file",
    )?;
    let mut created = crate::layer_merge::preview(&d.snapshot(), 0)?;
    let mut scene = created.scene.clone();
    scene.objects.push(crate::scene::PageObject::Line {
        start: [1., 1.],
        end: [3., 1.],
        width_px: 1.,
        color: [30, 60, 90, 128],
    });
    created.edit_scene(scene)?;
    let created_id = created.scene.catalog.objects.id(0);
    let order = crate::render_order::normalized(&created.scene);
    check(
        order.iter().position(|e| e.key == Key::Object(created_id))
            < order.iter().position(|e| e.key == selected),
        "New source creation lost mixed raster compositing order",
    )?;
    let a = Renderer::new(&d);
    let b = Renderer::new(&reopened);
    for y in 0..d.height {
        for x in 0..d.width {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            check(
                a.pixel(p) == b.pixel(p),
                "Raster order changed after reopen",
            )?;
        }
    }
    check(
        !crate::psd::layers_supported(&d),
        "PSD layer export silently lost raster substitute order",
    )?;
    check(
        d.undo()
            && d.undo()
            && d.undo()
            && d.snapshot() == original
            && d.redo()
            && d.redo()
            && d.redo()
            && d.snapshot() == after,
        "Rasterization Undo/Redo lost editable sources",
    )?;
    let dup = crate::layers::prepare(
        &d,
        crate::layers::Command::Duplicate(crate::layers::Target::Raster(id)),
    )?;
    let crate::layers::Target::Raster(new) = dup.selected else {
        return Err("Raster duplicate kind".into());
    };
    check(
        dup.state.scene.render_order.deferred(new),
        "Raster duplicate lost source position",
    )?;
    let moving = crate::layers::prepare(
        &d,
        crate::layers::Command::Move {
            target: crate::layers::Target::Raster(id),
            parent: None,
            position: 0,
        },
    )?;
    check(
        !moving.state.scene.render_order.deferred(id),
        "Explicit raster-stack move retained source slot",
    )?;
    let f = crate::layer_merge::prepare(
        &d,
        crate::layer_merge::Operation::Flatten,
        Default::default(),
    )?;
    check(
        f.state.scene.render_order.is_default(),
        "Flatten retained raster substitute order",
    )?;
    for (expr, expected) in [
        (Expression::Gray, [117, 117, 117, 128]),
        (Expression::Mono, [0, 0, 0, 128]),
    ] {
        let mut sample = crate::layer_roles_verify::page()?;
        sample.layers[0].pixels[0] = [200, 100, 50, 128];
        let state = core::prepare(
            &sample,
            Key::Layer(Target::Raster(1)),
            Action::Rasterize(expr),
        )?;
        sample.commit_state(state.state)?;
        check(
            sample.layers[0].pixels[0] == expected,
            "Raster expression conversion or original alpha",
        )?;
    }
    clipped()?;
    raster_effects()?;
    color_edits(root)?;
    vector_color()?;
    Ok(())
}
fn clipped() -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    let base = d.scene.lines.add_layer("Base")?;
    d.scene.lines.add_path(
        base,
        vec![[1., 3.], [6., 3.]],
        false,
        3.,
        [80, 100, 120, 128],
    )?;
    let top = d.scene.lines.add_layer("Clip")?;
    d.scene
        .lines
        .add_path(top, vec![[1., 3.], [6., 3.]], false, 1., [200, 40, 10, 192])?;
    d.scene
        .clipping
        .bind(Target::Vector(top), Some(Target::Vector(base)));
    let mut bad = d.scene.clone();
    bad.render_order.entries = vec![
        crate::render_order::Entry {
            key: Key::Layer(Target::Vector(base)),
            group: crate::render_order::Group::Vector,
        },
        crate::render_order::Entry {
            key: Key::Layer(Target::Raster(1)),
            group: crate::render_order::Group::Vector,
        },
        crate::render_order::Entry {
            key: Key::Layer(Target::Vector(top)),
            group: crate::render_order::Group::Vector,
        },
    ];
    check(
        d.edit_scene(bad).is_err(),
        "Native raster order split a vector clipping run",
    )?;
    let before = d.snapshot();
    let pixel = Renderer::new(&d).pixel([3.5, 3.5]);
    let p = core::prepare(
        &d,
        Key::Layer(Target::Vector(top)),
        Action::Rasterize(Expression::Color),
    )?;
    check(
        p.affected.len() == 2,
        "Partial vector clipping run rasterized independently",
    )?;
    d.commit_state(p.state)?;
    check(
        d.scene.lines.layers.is_empty()
            && Renderer::new(&d).pixel([3.5, 3.5]) == pixel
            && pixel[3] == 128
            && d.undo()
            && d.snapshot() == before,
        "Rasterized clip increased alpha or lost Undo",
    )?;
    Ok(())
}
fn raster_effects() -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    d.layers[0].pixels[27] = [200, 80, 40, 128];
    d.scene.appearance.rasters.insert(
        1,
        crate::appearance::Style {
            enabled: true,
            border: 1,
            border_color: [30, 60, 90, 128],
            ..Default::default()
        },
    );
    d.scene.layer_properties.rasters.insert(
        1,
        p::Properties {
            opacity: 60,
            ..Default::default()
        },
    );
    let mask = crate::layer_folders::rectangle_mask(
        &d,
        crate::scene::Rect {
            min: [0.; 2],
            max: [8.; 2],
        },
        128,
    )?;
    d.scene.layer_effects.rasters.entry(1).or_default().mask = Some(mask);
    let original = d.snapshot();
    let p = core::prepare(
        &d,
        Key::Layer(Target::Raster(1)),
        Action::Rasterize(Expression::Color),
    )?;
    check(p.maximum_delta <= 1, "Raster effects changed composite")?;
    d.commit_state(p.state)?;
    check(
        d.scene.appearance.rasters.is_empty()
            && d.scene.layer_effects.rasters.is_empty()
            && p::properties(&d, Target::Raster(1)).opacity == 60
            && d.undo()
            && d.snapshot() == original,
        "Effect bake did not preserve opacity/editable Undo",
    )?;
    Ok(())
}
fn color_edits(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    d.layers[0].pixels[0] = [80, 120, 160, 128];
    d.layers[0].pixels[1] = [40, 80, 120, 64];
    d.layers[0].pixels[2] = [70, 80, 90, 0];
    d.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 0,
            y: 0,
            length: 1,
            alpha: 128,
        }],
    });
    let before = d.snapshot();
    let state = core::prepare(
        &d,
        Key::Layer(Target::Raster(1)),
        Action::Color {
            color: [200, 20, 60, 10],
            selection: true,
            paths: vec![],
        },
    )?;
    d.commit_state(state.state)?;
    check(
        d.layers[0].pixels[..3] == [[140, 70, 110, 128], [40, 80, 120, 64], [0; 4]]
            && d.undo()
            && d.snapshot() == before,
        "Selection RGB mix/old alpha/zero-alpha canonicalization",
    )?;
    d.layers[0].pixels[..5].copy_from_slice(&[
        [0, 0, 0, 255],
        [128, 128, 128, 255],
        [255, 255, 255, 128],
        [128, 128, 128, 128],
        [64, 64, 64, 64],
    ]);
    let before = d.snapshot();
    let prepared = core::prepare(
        &d,
        Key::Layer(Target::Raster(1)),
        Action::Brightness { selection: false },
    )?;
    d.commit_state(prepared.state)?;
    check(
        d.layers[0].pixels[..5]
            == [
                [0, 0, 0, 255],
                [0, 0, 0, 127],
                [0; 4],
                [0, 0, 0, 64],
                [0, 0, 0, 48],
            ],
        "Independent brightness/alpha gray staircase",
    )?;
    let path = root.join("brightness-opacity.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.undo()
            && d.snapshot() == before,
        "Brightness native/one Undo",
    )?;
    let mut sample = crate::layer_roles_verify::page()?;
    sample.layers[0].pixels[0] = [128, 128, 128, 128];
    sample.layers[0].pixels[1] = [255, 255, 255, 128];
    sample.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 0,
            y: 0,
            length: 2,
            alpha: 128,
        }],
    });
    let mixed = core::prepare(
        &sample,
        Key::Layer(Target::Raster(1)),
        Action::Brightness { selection: true },
    )?;
    check(
        mixed.state.layers[0].pixels[..2] == [[85, 85, 85, 96], [255, 255, 255, 64]],
        "Partial brightness premultiplication changed white or alpha",
    )?;
    let c = crate::adjustments::prepare(
        &d,
        crate::adjustments::Destination::New {
            name: "Correction".into(),
            parent: None,
        },
        Default::default(),
    )?;
    let id = c.layer;
    d.commit_state(c.state)?;
    let before = d.snapshot();
    check(
        core::prepare(
            &d,
            Key::Layer(Target::Raster(id)),
            Action::Color {
                color: [255; 4],
                selection: false,
                paths: vec![],
            },
        )
        .is_err()
            && d.snapshot() == before,
        "Recolor accepted correction content",
    )?;
    Ok(())
}
fn vector_color() -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    let layer = d.scene.lines.add_layer("Pressure outline")?;
    let selected = d.scene.lines.add_path(
        layer,
        vec![
            [1., 4.],
            [2., 2.],
            [4., 1.],
            [6., 3.],
            [6., 4.],
            [4., 3.],
            [2., 4.],
        ],
        true,
        1.,
        [80, 120, 160, 128],
    )?;
    let other =
        d.scene
            .lines
            .add_path(layer, vec![[0., 7.], [7., 7.]], false, 1., [10, 20, 30, 64])?;
    let path = d.scene.lines.layers[0]
        .paths
        .iter_mut()
        .find(|p| p.id == selected)
        .unwrap();
    path.fill = Some([80, 120, 160, 64]);
    path.stroke = false;
    let before = d.snapshot();
    let prepared = core::prepare(
        &d,
        Key::Layer(Target::Vector(layer)),
        Action::Color {
            color: [200, 20, 60, 2],
            selection: true,
            paths: vec![selected],
        },
    )?;
    d.commit_state(prepared.state)?;
    let mut p = d.scene.lines.layers[0].paths[0].clone();
    check(
        p.color == [200, 20, 60, 128]
            && p.fill == Some([200, 20, 60, 64])
            && d.scene.lines.layers[0].paths[1].id == other
            && d.scene.lines.layers[0].paths[1] == before.scene.lines.layers[0].paths[1],
        "Selected pressure-outline/stroke color or old alpha",
    )?;
    p.color = before.scene.lines.layers[0].paths[0].color;
    p.fill = before.scene.lines.layers[0].paths[0].fill;
    check(
        p == before.scene.lines.layers[0].paths[0] && d.undo() && d.snapshot() == before,
        "Recolor changed geometry/pressure outline/Undo",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_raster_order_clip_alpha_effects_brightness_color_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
