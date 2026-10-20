use crate::{
    appearance::Style,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    expression_color::{self as e, Settings},
    layer_properties::Target,
    layer_search::Expression,
    render::Renderer,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let settings = Settings::default();
    for value in 0..=255 {
        for alpha in [0, 1, 127, 128, 129, 255] {
            let source = [value, value, value, alpha];
            let gray = if alpha == 0 { [0; 4] } else { source };
            check(
                e::pixel(source, Expression::Gray, settings) == gray,
                "Gray neutral/alpha boundary",
            )?;
            for threshold in [value.saturating_sub(1), value, value.saturating_add(1)] {
                let white = if value >= threshold { 255 } else { 0 };
                let expected = if alpha == 0 || alpha < 128 {
                    [0; 4]
                } else {
                    [white, white, white, 255]
                };
                check(
                    e::pixel(
                        source,
                        Expression::Mono,
                        Settings {
                            brightness: threshold,
                            ..settings
                        },
                    ) == expected,
                    "Monochrome >= brightness/alpha boundary",
                )?;
            }
            for threshold in [alpha.saturating_sub(1), alpha, alpha.saturating_add(1)] {
                let value = if value >= 128 { 255 } else { 0 };
                let expected = if alpha == 0 || alpha < threshold {
                    [0; 4]
                } else {
                    [value, value, value, 255]
                };
                check(
                    e::pixel(
                        source,
                        Expression::Mono,
                        Settings {
                            alpha: threshold,
                            ..settings
                        },
                    ) == expected,
                    "Monochrome alpha threshold endpoints",
                )?;
            }
        }
    }
    for (source, gray) in [
        ([255, 0, 0, 128], 127),
        ([0, 255, 0, 1], 220),
        ([0, 0, 255, 255], 76),
    ] {
        check(
            e::pixel(source, Expression::Gray, settings) == [gray, gray, gray, source[3]],
            "Linear luminance independent primary patch",
        )?;
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels = vec![
        [127, 127, 127, 127],
        [128, 128, 128, 128],
        [129, 129, 129, 129],
        [240, 30, 50, 0],
    ];
    let target = Target::Raster(1);
    let original = d.snapshot();
    let history = d.history_counts();
    let state = e::prepare_display(&d, target, Style::default(), Expression::Mono, settings)?;
    check(
        d.snapshot() == original && d.history_counts() == history,
        "Live preview mutated original/Undo",
    )?;
    d.commit_state(state)?;
    let live = d.snapshot();
    let expected = vec![[0; 4], [255; 4], [255; 4], [0; 4]];
    check(
        Renderer::new(&d).output(300., 1024)?.2 == expected && d.layers == original.layers,
        "Live thresholds altered raw ink or rendered wrong boundaries",
    )?;
    let file = root.join("expression-thresholds.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let opened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        opened.snapshot() == live && Renderer::new(&opened).output(300., 1024)?.2 == expected,
        "Threshold settings native reopen",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&59u32.to_le_bytes());
    let old = root.join("expression-old-version.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Older header accepted new expression thresholds",
    )?;
    check(
        d.undo() && d.snapshot() == original && d.redo() && d.snapshot() == live,
        "Live threshold Undo/Redo",
    )?;
    let converted = e::prepare_pixels(&d, target, Style::default(), Expression::Mono, settings)?;
    check(
        d.snapshot() == live,
        "Actual conversion preview mutated source",
    )?;
    d.commit_state(converted)?;
    check(
        d.layers[0].pixels == expected
            && crate::appearance::expression(&d, target) == Expression::Color
            && d.scene.appearance.is_default(),
        "Actual conversion left display conversion active or lost pixels",
    )?;
    let actual_file = root.join("expression-actual.drawer");
    let actual = d.snapshot();
    crate::files::save(&mut d, &actual_file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&actual_file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == actual,
        "Actual expression samples native reopen",
    )?;
    let png = root.join("expression-alpha.png");
    crate::png_export::export(&d, &png, 1024 * 1024).map_err(|e| e.to_string())?;
    let image = image::ImageReader::open(png)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .into_rgba8();
    check(
        image.pixels().map(|p| p.0).collect::<Vec<_>>() == expected,
        "Expression output threshold alpha",
    )?;
    check(
        d.undo() && d.snapshot() == live,
        "Actual conversion did not restore raw ink and live thresholds together",
    )?;
    d.scene.clipping.alpha.insert(target);
    let locked = d.snapshot();
    check(
        e::prepare_pixels(&d, target, Style::default(), Expression::Mono, settings).is_err()
            && d.snapshot() == locked,
        "Actual threshold conversion bypassed alpha protection",
    )?;
    d.scene.clipping.alpha.clear();
    d.scene
        .layer_properties
        .rasters
        .entry(1)
        .or_default()
        .pixels_locked = true;
    check(
        e::prepare_pixels(&d, target, Style::default(), Expression::Gray, settings).is_err()
            && e::prepare_display(&d, target, Style::default(), Expression::Gray, settings).is_ok(),
        "Pixel lock/live display separation",
    )?;
    d.scene.layer_properties.rasters.clear();
    let id = d.scene.lines.add_layer("Threshold vector")?;
    let path = d.scene.lines.add_path(
        id,
        vec![[0., 0.], [3., 0.], [3., 1.]],
        true,
        1.,
        [127, 127, 127, 128],
    )?;
    d.scene.lines.layers[0].paths[0].fill = Some([128; 4]);
    let source = d.snapshot();
    let converted = e::prepare_pixels(
        &d,
        Target::Vector(id),
        Style::default(),
        Expression::Mono,
        settings,
    )?;
    d.commit_state(converted)?;
    let p = &d.scene.lines.layers[0].paths[0];
    check(
        p.id == path
            && p.points == source.scene.lines.layers[0].paths[0].points
            && p.color == [0, 0, 0, 255]
            && p.fill == Some([255; 4]),
        "Vector stored stroke/fill conversion changed geometry or threshold result",
    )?;
    check(
        d.undo() && d.snapshot() == source,
        "Vector expression conversion Undo",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn expression_threshold_boundaries_live_pixels_locks_vector_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
