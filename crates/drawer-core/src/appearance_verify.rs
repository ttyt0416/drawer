use crate::{
    appearance::{self as core, Style},
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_properties::Target,
    render::Renderer,
};
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
fn make() -> Result<Document, String> {
    let mut d = crate::layer_roles_verify::page()?;
    d.layers[0].pixels[3 * 8 + 3] = [200, 100, 50, 128];
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    let t = Target::Raster(1);
    let original = d.snapshot();
    let style = Style {
        enabled: true,
        color: Some([20, 40, 60]),
        border: 1,
        rim: 0.5,
        ..Default::default()
    };
    let state = core::prepare(&d, t, style.clone())?;
    check(d.snapshot() == original, "Display prepare changed source")?;
    d.commit_state(state)?;
    let rendered = Renderer::new(&d);
    check(
        rendered.pixel([3.5; 2]) == [10, 20, 30, 128]
            && rendered.pixel([2.5, 3.5]) == [0, 0, 0, 128]
            && rendered.pixel([2.5; 2]) == [0, 0, 0, 128]
            && rendered.pixel([1.5, 3.5]) == [0; 4],
        "Independent display color/rim/square border pixels",
    )?;
    check(
        d.layers[0].pixels == original.layers[0].pixels,
        "Display recipe changed original RGBA",
    )?;
    let unchanged = d.snapshot();
    let counts = d.history_counts();
    let png = root.join("display-effects.png");
    crate::png_export::export(&d, &png, 1024 * 1024).map_err(|e| e.to_string())?;
    let image = image::ImageReader::open(&png)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .into_rgba8();
    check(
        image.get_pixel(3, 3).0 == [10, 20, 30, 128] && image.get_pixel(2, 3).0 == [0, 0, 0, 128],
        "PNG omitted live display effects",
    )?;
    let psd = root.join("display-effects.psd");
    crate::psd::export(
        &d,
        &psd,
        crate::psd::Choice::RenderedLayers,
        false,
        1024 * 1024,
    )
    .map_err(|e| e.to_string())?;
    let psd = crate::psd::open(&psd, 1024 * 1024).map_err(|e| e.to_string())?;
    check(
        psd.layers[0].pixels[3 * 8 + 3] == [10, 20, 30, 128]
            && psd.layers[0].pixels[3 * 8 + 2] == [0, 0, 0, 128]
            && d.snapshot() == unchanged
            && d.history_counts() == counts,
        "PSD display effect baking changed source or pixels",
    )?;
    let source = d.snapshot();
    let state = core::prepare(
        &d,
        t,
        Style {
            enabled: false,
            ..style.clone()
        },
    )?;
    d.commit_state(state)?;
    check(
        Renderer::new(&d).pixel([3.5; 2]) == [200, 100, 50, 128]
            && Renderer::new(&d).pixel([2.5, 3.5]) == [0; 4],
        "Disabled display recipe did not restore raw appearance",
    )?;
    check(d.undo() && d.snapshot() == source, "Display change Undo")?;
    let state = core::prepare(&d, t, Style::default())?;
    d.commit_state(state)?;
    let mut scene = d.scene.clone();
    scene.selection = Some(crate::scene::Rect {
        min: [3.; 2],
        max: [4.; 2],
    });
    d.edit_scene(scene)?;
    let scope = core::capture_scope(&d)?;
    let state = core::prepare(
        &d,
        t,
        Style {
            scope: Some(scope),
            ..style.clone()
        },
    )?;
    d.commit_state(state)?;
    check(
        Renderer::new(&d).pixel([3.5; 2]) == [10, 20, 30, 128]
            && Renderer::new(&d).pixel([2.5, 3.5]) == [0; 4],
        "Display recipe escaped frozen area selection",
    )?;
    let before = d.snapshot();
    let state = crate::precision::Nudge::begin(&d, 0, crate::precision::Target::Layer)?
        .preview_total(&d, [2, 0])?;
    d.commit_state(state)?;
    check(
        Renderer::new(&d).pixel([5.5, 3.5]) == [10, 20, 30, 128]
            && d.scene.appearance.rasters[&1]
                .scope
                .as_ref()
                .is_some_and(|s| s.translation == [2., 0.]),
        "Frozen effect selection did not follow layer movement",
    )?;
    check(
        d.undo() && d.snapshot() == before,
        "Moved display scope Undo",
    )?;
    let file = root.join("layer-appearance.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot()
            && Renderer::new(&reopened).pixel([3.5; 2]) == [10, 20, 30, 128],
        "Display recipe/scope native reopen",
    )?;
    let mut edge = crate::layer_roles_verify::page()?;
    edge.layers[0].pixels[0] = [200, 100, 50, 128];
    let state = core::prepare(
        &edge,
        t,
        Style {
            enabled: true,
            border: 2,
            ..Default::default()
        },
    )?;
    edge.commit_state(state)?;
    let before = edge.snapshot();
    crate::paper::bake(&mut edge, crate::paper::Node::Raster(0))?;
    check(
        edge.layers[0].extent(8, 8).offset == [-2, -2]
            && edge.layers[0].pixel(8, 8, -2, -2) == [0, 0, 0, 128]
            && edge.scene.appearance.is_default(),
        "Off-canvas border was cropped or applied twice while baking",
    )?;
    check(
        edge.undo() && edge.snapshot() == before,
        "Display bake Undo",
    )?;
    check(
        crate::layer_pick::candidates(&edge, [2.5; 2], Default::default()).first()
            == Some(&crate::thumbnail::Target::Raster(0)),
        "Border pixels were not canvas-pick candidates",
    )?;
    let state = crate::layers::prepare(
        &edge,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "blank".into(),
            parent: None,
        },
    )?
    .state;
    edge.commit_state(state)?;
    let before = Renderer::new(&edge).output(300., 1024 * 1024)?.2;
    let merged = crate::layer_merge::prepare(
        &edge,
        crate::layer_merge::Operation::Selected(vec![Target::Raster(1), Target::Raster(2)]),
        Default::default(),
    )?;
    edge.commit_state(merged.state)?;
    let after = Renderer::new(&edge).output(300., 1024 * 1024)?.2;
    check(
        before
            .iter()
            .zip(after)
            .all(|(a, b)| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1))
            && edge.scene.appearance.is_default(),
        "Merge failed to bake display recipe",
    )?;
    let mut vector = crate::layer_roles_verify::page()?;
    let mut scene = vector.scene.clone();
    let id = scene.lines.add_layer("Display vector")?;
    scene
        .lines
        .add_path(id, vec![[2., 3.], [5., 3.]], false, 1., [200, 100, 50, 255])?;
    vector.edit_scene(scene)?;
    let raw = vector.scene.lines.clone();
    let s = core::prepare(
        &vector,
        Target::Vector(id),
        Style {
            enabled: true,
            color: Some([10, 200, 30]),
            border: 1,
            ..Default::default()
        },
    )?;
    vector.commit_state(s)?;
    check(
        vector.scene.lines == raw
            && Renderer::new(&vector).pixel([3.5, 3.5]) == [10, 200, 30, 255]
            && crate::layer_pick::candidates(&vector, [3.5, 4.5], Default::default()).first()
                == Some(&crate::thumbnail::Target::Vector(id)),
        "Vector display recipe changed ink or missed outline pick",
    )?;
    let before = vector.snapshot();
    for style in [
        Style {
            border: 33,
            ..Default::default()
        },
        Style {
            rim: f64::NAN,
            ..Default::default()
        },
    ] {
        check(
            core::prepare(&vector, Target::Vector(id), style).is_err()
                && vector.snapshot() == before,
            "Invalid display recipe modified source",
        )?;
    }
    let gray = core::prepare_expression(
        &d,
        t,
        Style::default(),
        Some(crate::layer_search::Expression::Gray),
    )?;
    d.commit_state(gray)?;
    check(
        d.layers[0].pixels[3 * 8 + 3] == [200, 100, 50, 128]
            && Renderer::new(&d).pixel([3.5; 2]) == [117, 117, 117, 128],
        "Nondestructive expression color rendering",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn display_color_border_rim_scope_move_bake_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
