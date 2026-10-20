use crate::{
    document::DEFAULT_DOCUMENT_BUDGET,
    material_layer::{self as core, Image, Layer, Source},
    render::Renderer,
    scene::{Affine, PageObject, Rect},
};
fn check(ok: bool, s: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(s.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = crate::layer_roles_verify::page()?;
    let mut layer = Layer {
        source: Source::Solid([12, 34, 56, 128]),
        area: Rect {
            min: [0.; 2],
            max: [8.; 2],
        },
        transform: Affine::default(),
        repeat: [false; 2],
        mapping: Default::default(),
    };
    let original = d.snapshot();
    let counts = d.history_counts();
    let (s, id) = core::prepare(&d, None, "Fill", layer.clone())?;
    check(
        d.snapshot() == original,
        "Material prepare changed document",
    )?;
    d.commit_state(s)?;
    check(
        d.history_counts().0 == counts.0 + 1
            && Renderer::new(&d).pixel([2.5; 2]) == [12, 34, 56, 128],
        "Solid material / one Undo",
    )?;
    check(
        d.undo() && d.snapshot() == original && d.redo(),
        "Material Undo/Redo",
    )?;
    let g = crate::gradient::Settings {
        start: [0.5, 0.5],
        end: [2.5, 0.5],
        first: [0, 0, 0, 255],
        last: [200, 100, 50, 255],
        ..Default::default()
    };
    layer.source = Source::Gradient(Box::new(g));
    let (s, _) = core::prepare(&d, Some(id), "Gradient", layer.clone())?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([1.5, 0.5]) == [100, 50, 25, 255],
        "Independent gradient midpoint",
    )?;
    check(
        crate::layer_catalog::rows(&d)?.iter().any(|r| {
            r.key == crate::layer_search::Key::Object(id)
                && r.kind == crate::layer_search::Kind::Gradient
        }),
        "Material search kind",
    )?;
    let pixels = std::sync::Arc::new(vec![[230, 20, 40, 128], [0, 0, 0, 0]]);
    layer.source = Source::Image(Image {
        width: 2,
        height: 1,
        pixels: pixels.clone(),
    });
    let (s, _) = core::prepare(&d, Some(id), "Image", layer.clone())?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([0.5, 0.5]) == [230, 20, 40, 128]
            && Renderer::new(&d).pixel([1.5, 0.5]) == [0; 4]
            && Renderer::new(&d).pixel([2.5, 0.5]) == [0; 4]
            && Renderer::new(&d).pixel([0.5, 1.5]) == [0; 4],
        "Small transparent native-sized image",
    )?;
    layer.repeat = [true; 2];
    let (s, _) = core::prepare(&d, Some(id), "Tiled", layer.clone())?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([6.5, 5.5]) == [230, 20, 40, 128]
            && Renderer::new(&d).pixel([7.5, 5.5]) == [0; 4],
        "Independent image XY repeat",
    )?;
    layer.repeat = [false; 2];
    layer.transform = Affine([0., 2., -2., 0., 4., 1.]);
    let (s, _) = core::prepare(&d, Some(id), "Rotated", layer.clone())?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([3., 2.]) == [230, 20, 40, 128]
            && Renderer::new(&d).pixel([3., 4.]) == [0; 4],
        "Rotate/scale/move image",
    )?;
    let before = d.snapshot();
    d.view.angle = 1.2;
    check(
        d.snapshot() == before && Renderer::new(&d).pixel([3., 2.]) == [230, 20, 40, 128],
        "Material view independence",
    )?;
    let path = root.join("material.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot()
            && Renderer::new(&reopened).pixel([3., 2.]) == [230, 20, 40, 128],
        "Native embedded image/recipe roundtrip",
    )?;
    let PageObject::Material(m) = &reopened.scene.objects[0] else {
        return Err("Missing material".into());
    };
    check(
        matches!(&m.source,Source::Image(i)if i.pixels.as_ref()==pixels.as_ref()),
        "Transformed original bitmap changed",
    )?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    for n in [bytes.len() - 1, bytes.len() - 8] {
        std::fs::write(root.join("truncated-material.drawer"), &bytes[..n])
            .map_err(|e| e.to_string())?;
        check(
            crate::files::open(
                &root.join("truncated-material.drawer"),
                DEFAULT_DOCUMENT_BUDGET,
            )
            .is_err(),
            "Truncated image accepted",
        )?;
    }
    let mut corrupt = bytes.clone();
    let len = corrupt.len();
    corrupt[len - 16] = 255;
    std::fs::write(root.join("corrupt-material.drawer"), &corrupt).map_err(|e| e.to_string())?;
    check(
        crate::files::open(
            &root.join("corrupt-material.drawer"),
            DEFAULT_DOCUMENT_BUDGET,
        )
        .is_err()
            && crate::files::open(&path, 16).is_err(),
        "Invalid payload size/budget accepted",
    )?;
    let old = d.snapshot();
    layer.transform = Affine([0.; 6]);
    check(
        core::prepare(&d, Some(id), "Invalid", layer).is_err() && d.snapshot() == old,
        "Invalid transform changed source",
    )?;
    let mut bad = d.snapshot();
    if let PageObject::Material(m) = &mut bad.scene.objects[0]
        && let Source::Image(i) = &mut m.source
    {
        i.pixels = std::sync::Arc::new(vec![]);
    }
    check(
        d.commit_state(bad).is_err() && std::fs::read(&path).map_err(|e| e.to_string())? == bytes,
        "Incomplete bitmap committed or damaged save",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn materials_repeat_transform_transparency_undo_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
