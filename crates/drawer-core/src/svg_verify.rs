use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    svg_export as export, svg_import as import,
};
use std::path::Path;
pub fn page() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn reference(text: &str, w: u32, h: u32) -> Result<Vec<[u8; 4]>, String> {
    let (tree, _) = import::parse(text, None)?;
    let mut bitmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("Reference SVG dimensions")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut bitmap.as_mut(),
    );
    Ok(bitmap
        .pixels()
        .iter()
        .map(|p| {
            let p = p.demultiply();
            [p.red(), p.green(), p.blue(), p.alpha()]
        })
        .collect())
}
fn compare(d: &Document, text: &str, label: &str) -> Result<(), String> {
    let raster = text.contains("<image");
    let scale = if raster { 1 } else { 8 };
    let (tree, _) = import::parse(text, None)?;
    let mut bitmap = resvg::tiny_skia::Pixmap::new(d.width * scale, d.height * scale)
        .ok_or("Reference SVG size")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
        &mut bitmap.as_mut(),
    );
    let renderer = crate::render::Renderer::for_export(d)?;
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    for y in 0..d.height {
        for x in 0..d.width {
            let mut sum = [0.; 4];
            let mut target = [0.; 4];
            for sy in 0..scale {
                for sx in 0..scale {
                    let p = renderer.pixel([
                        x as f64 + (sx as f64 + 0.5) / scale as f64,
                        y as f64 + (sy as f64 + 0.5) / scale as f64,
                    ]);
                    for i in 0..3 {
                        sum[i] += p[i] as f64 * p[3] as f64 / 255.;
                    }
                    sum[3] += p[3] as f64;
                    let q = bitmap.pixel(x * scale + sx, y * scale + sy).unwrap();
                    target[0] += q.red() as f64;
                    target[1] += q.green() as f64;
                    target[2] += q.blue() as f64;
                    target[3] += q.alpha() as f64;
                }
            }
            let actual = sum.map(|v| v / (scale * scale) as f64);
            let target = target.map(|v| v / (scale * scale) as f64);
            // Boolean native coverage and the independent reader's analytic AA
            // can differ along sampled outline edges. Interior probes below
            // verify paint/alpha much more tightly.
            if actual.iter().zip(target).any(|(a, b)| (a - b).abs() > 26.) {
                return Err(format!(
                    "{label}: independent SVG pixel mismatch at{x},{y}: native={actual:?}, SVG={target:?}"
                ));
            }
        }
    }
    Ok(())
}
const BASIC: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><g transform="translate(4 3)"><path d="m2 2 h20 v20 h-20 z" fill="#d02040" fill-opacity="0.6"/><path d="M4 37 Q14 20 25 38 T48 38 A5 5 0 0 1 53 43" fill="none" stroke="#2060d0" stroke-width="3" stroke-linecap="square" stroke-linejoin="bevel" stroke-dasharray="6 3"/></g></svg>"##;
const GRADIENT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><defs><linearGradient id="linear" x1="0" x2="1"><stop stop-color="#f02010"/><stop offset="1" stop-color="#2040e0" stop-opacity="0.7"/></linearGradient><radialGradient id="radial" cx="0.5" cy="0.5" r="0.5" fx="0.3" fy="0.4"><stop stop-color="#ffffff"/><stop offset="1" stop-color="#20a050"/></radialGradient><clipPath id="clip"><rect x="8" y="7" width="32" height="22"/><circle cx="42" cy="20" r="8"/></clipPath></defs><g transform="translate(2 3)" clip-path="url(#clip)"><path d="M2 2 H52 V32 H2 Z M15 12 V22 H26 V12 Z" fill="url(#linear)" fill-rule="evenodd"/></g><ellipse cx="34" cy="46" rx="20" ry="10" fill="url(#radial)"/></svg>"##;
pub fn verify(root: &Path) -> Result<(), String> {
    for (name, svg) in [("basic", BASIC), ("gradient-clip-hole", GRADIENT)] {
        let mut d = page()?;
        let before = d.snapshot();
        let history = d.history_counts();
        let prepared = import::prepare(&d, svg, None).map_err(|e| format!("{name} import: {e}"))?;
        check(
            prepared.paths > 0 && d.snapshot() == before && d.history_counts() == history,
            "SVG prepare changed source/Undo",
        )?;
        d.commit_state(prepared.state)?;
        check(
            d.history_counts().0 == history.0 + 1,
            "SVG import created multiple Undo steps",
        )?;
        compare(&d, svg, name)?;
        let exact = reference(svg, d.width, d.height)?;
        let renderer = crate::render::Renderer::for_export(&d)?;
        let probes = if name == "basic" {
            vec![(10, 10), (2, 2)]
        } else {
            vec![(12, 12), (45, 46), (22, 19), (5, 5)]
        };
        for (x, y) in probes {
            let a = renderer.pixel([x as f64 + 0.5, y as f64 + 0.5]);
            let b = exact[(y * d.width + x) as usize];
            check(
                a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2),
                &format!("SVG interior paint/alpha probe {name} {x},{y}: {a:?}, {b:?}"),
            )?;
        }
        let before_export = d.snapshot();
        let exported = export::generate(&d)?;
        compare(&d, &exported.text, "SVG export normalized paints")?;
        check(
            d.snapshot() == before_export,
            "SVG export changed editable source",
        )?;
        let xml = usvg::roxmltree::Document::parse(&exported.text).map_err(|e| e.to_string())?;
        check(
            xml.root_element().attribute("width") == Some("64")
                && xml.root_element().attribute("height") == Some("64"),
            "SVG export canvas size",
        )?;
        check(
            xml.descendants()
                .flat_map(|n| n.attributes())
                .filter(|a| a.name() == "href")
                .all(|a| {
                    a.value().starts_with("data:image/png;base64,") || a.value().starts_with('#')
                }),
            "SVG export generated an external resource",
        )?;
        let native = root.join(format!("svg-{name}.drawer"));
        crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "SVG native paint/clip roundtrip",
        )?;
        let mut bytes = std::fs::read(&native).map_err(|e| e.to_string())?;
        bytes[8..12].copy_from_slice(&67u32.to_le_bytes());
        std::fs::write(&native, bytes).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "SVG paint accepted under old header",
        )?;
        check(
            d.undo() && d.snapshot() == before,
            "SVG import Undo did not restore page",
        )?;
        let saved = root.join(format!("{name}.svg"));
        export::write(&saved, &exported.text)?;
        let first = std::fs::read(&saved).map_err(|e| e.to_string())?;
        export::write(&saved, &exported.text)?;
        check(
            first == std::fs::read(saved).map_err(|e| e.to_string())?,
            "SVG overwrite changed generated bytes",
        )?;
    }
    malformed()?;
    outlines()?;
    resources(root).map_err(|e| format!("SVG resources: {e}"))?;
    additional(root)?;
    Ok(())
}
fn additional(root: &Path) -> Result<(), String> {
    for (fill_rule, alpha) in [("evenodd", 0), ("nonzero", 255)] {
        let text = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><path d="M4 4H28V28H4Z M10 10H22V22H10Z" fill="#e03050" fill-rule="{fill_rule}"/></svg>"##
        );
        let mut d = page()?;
        import::prepare(&d, &text, None)?.state.restore(&mut d);
        check(
            crate::render::Renderer::new(&d).pixel([16.5, 16.5])[3] == alpha,
            "SVG fill rule did not preserve same-direction holes",
        )?;
        compare(&d, &export::generate(&d)?.text, "SVG fill rule export")?;
    }
    let transformed = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 32 32"><g transform="translate(4 4)"><g transform="rotate(10) scale(1.1 .7)"><rect width="18" height="20" fill="#4080d0" fill-opacity=".6"/></g></g></svg>"##;
    let mut d = page()?;
    d.scene.transform = crate::scene::Affine([1.2, 0.1, 0.2, 1.1, 0., 0.]);
    import::prepare(&d, transformed, None)?
        .state
        .restore(&mut d);
    compare(
        &d,
        transformed,
        "SVG viewBox, nested and destination transforms",
    )?;
    compare(&d, &export::generate(&d)?.text, "SVG transformed export")?;
    for text in [
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><g transform="translate(6 7)" opacity=".6"><rect x="2" y="2" width="24" height="20" fill="#d04020"/><rect x="12" y="10" width="25" height="20" fill="#2050d0"/></g></svg>"##,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect x="4" y="4" width="54" height="54" fill="#c85020"/><g style="mix-blend-mode:multiply" opacity=".6"><rect x="12" y="12" width="32" height="30" fill="#2050d0"/></g></svg>"##,
    ] {
        let mut d = page()?;
        let p = import::prepare(&d, text, None)?;
        check(
            p.images == 1,
            "SVG isolated group did not record raster substitute",
        )?;
        d.commit_state(p.state)?;
        compare(&d, text, "SVG isolated opacity/blend")?;
        let file = root.join("svg-raster-native.drawer");
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "SVG raster substitute native placement/order/colors",
        )?;
        compare(&d, &export::generate(&d)?.text, "SVG isolated group export")?;
    }
    let mut d = page()?;
    import::prepare(&d, GRADIENT, None)?.state.restore(&mut d);
    let before = d.snapshot();
    let layer = d.scene.lines.layers[0].id;
    let state = crate::ruler_scope::translate_layer(
        &d,
        crate::line_tools::Target::Vector(layer),
        [3., 4.],
    )?;
    d.commit_state(state)?;
    let moved = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><g transform="translate(3 4)">{GRADIENT}</g></svg>"#
    );
    compare(&d, &moved, "SVG paint and clip follow native movement")?;
    check(d.undo() && d.snapshot() == before, "SVG movement Undo")?;
    Ok(())
}
fn malformed() -> Result<(), String> {
    let d = page()?;
    let before = d.snapshot();
    for text in [
        "<svg>",
        r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M broken"/></svg>"#,
        r#"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///no">]><svg xmlns="http://www.w3.org/2000/svg">&x;</svg>"#,
    ] {
        check(
            import::prepare(&d, text, None).is_err() && d.snapshot() == before,
            "Malformed SVG partially imported",
        )?;
    }
    let large = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><path d="M0 0 {} Z"/></svg>"#,
        (0..5000)
            .map(|i| format!("L{} {}", i % 64, (i * 7) % 64))
            .collect::<Vec<_>>()
            .join(" ")
    );
    check(
        import::prepare(&d, &large, None).is_err(),
        "Giant SVG path was not bounded",
    )?;
    let text = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1in" height="72pt"><text id="caption">ignored</text><script id="code">throw Error()</script><use href="#recursive"/></svg>"##;
    let p = import::prepare(&d, text, None)?;
    check(
        p.paths == 0
            && p.images == 0
            && p.state == before
            && p.warnings.len() >= 3
            && p.size == [96., 96.],
        "Units, unsupported item diagnostics or empty import source preservation",
    )
}
fn outlines() -> Result<(), String> {
    for tip in crate::vector_tip::Preset::ALL {
        let mut d = page()?;
        let layer = d.scene.lines.add_layer("SVG variable")?;
        d.scene.lines.add_path(
            layer,
            vec![[8., 12.], [27., 42.], [52., 12.]],
            false,
            7.,
            [30, 90, 210, 155],
        )?;
        let p = &mut d.scene.lines.layers[0].paths[0];
        p.tip = tip;
        p.pressure.device = vec![Some(0.4), None, Some(0.6)];
        p.pressure.controls = vec![0.5, 1.8, 1.];
        let before = d.snapshot();
        let out = export::generate(&d)?;
        check(
            out.warnings.iter().any(|w| w.contains("outlines")) && d.snapshot() == before,
            "Variable-width conversion warning/source preservation",
        )?;
        compare(&d, &out.text, &format!("Variable {tip:?} outline"))?;
    }
    Ok(())
}
fn resources(root: &Path) -> Result<(), String> {
    let png = root.join("local image.png");
    image::save_buffer_with_format(
        &png,
        &[230, 20, 40, 128, 20, 200, 40, 255],
        2,
        1,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?;
    let mut d = page()?;
    let text = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect x="2" y="2" width="10" height="10" fill="#f00"/><image x="10" y="10" width="24" height="12" href="local%20image.png"/><rect x="18" y="16" width="12" height="8" fill="#00f"/></svg>"##;
    let p = import::prepare(&d, text, Some(root))?;
    check(
        p.images == 1 && p.paths == 2,
        "Local image/vector mixed import",
    )?;
    d.commit_state(p.state)?;
    let out = export::generate(&d)?;
    check(
        out.text.contains("data:image/png;base64,") && !out.text.contains("local%20image.png"),
        &format!(
            "SVG image was not embedded; raster alpha counts={:?}; SVG bytes={}",
            d.layers
                .iter()
                .map(|l| l.pixels.iter().filter(|p| p[3] > 0).count())
                .collect::<Vec<_>>(),
            out.text.len()
        ),
    )?;
    compare(&d, &out.text, "Mixed vector/raster SVG export")?;
    let no_local = import::prepare(&page()?, text, None)?;
    check(
        no_local.images == 0 && no_local.warnings.iter().any(|w| w.contains("reference")),
        "Clipboard import accessed a local directory",
    )?;
    let dangerous = r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><image width="32" height="32" href="https://example.invalid/image.png"/><image width="32" height="32" href="../private.png"/><rect width="5" height="5"/></svg>"#;
    let p = import::prepare(&page()?, dangerous, Some(root))?;
    check(
        p.paths == 1 && p.images == 0 && p.warnings.len() >= 2,
        "Remote or parent-directory image was accessed",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn svg_import_export_independent_reader_bounds_resources_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
