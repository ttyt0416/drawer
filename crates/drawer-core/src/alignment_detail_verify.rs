use crate::{
    alignment::{self as a, Align, Item, Operation, Reference},
    alignment_detail::{self as detail, Options, Settings},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::{Affine, PageObject, Point, Rect},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7
}
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 64,
            height: 64,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    for (x, y, w, h) in [
        (2.0, 5.0, 4.0, 6.0),
        (17.0, 22.0, 10.0, 10.0),
        (44.0, 40.0, 16.0, 14.0),
    ] {
        d.scene.objects.push(PageObject::Tone {
            bounds: Rect {
                min: [x, y],
                max: [x + w, y + h],
            },
            period_px: 4.0,
            coverage: 0.5,
        });
    }
    let settings = Settings {
        selected: vec![Item::Object(0), Item::Object(1), Item::Object(2)],
        ..Default::default()
    };
    let old = d.snapshot();
    for (align, axis, t) in [
        (Align::Left, 0, 0.0),
        (Align::Center, 0, 0.5),
        (Align::Right, 0, 1.0),
        (Align::Top, 1, 0.0),
        (Align::Middle, 1, 0.5),
        (Align::Bottom, 1, 1.0),
    ] {
        d.commit_state(a::prepare_advanced(&d, &settings, Operation::Edges(align))?)?;
        let positions: Vec<_> = settings
            .selected
            .iter()
            .map(|i| {
                let r = a::bounds(&d, *i).unwrap();
                r.min[axis] * (1.0 - t) + r.max[axis] * t
            })
            .collect();
        check(
            (positions[1] - (positions[0] + positions[2]) / 2.0).abs() < 1e-8
                && d.scene.alignment == settings,
            "Independent edge/center distribution",
        )?;
        let file = root.join(format!("alignment-edge-{align:?}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot()
                && d.history_counts().0 == 1
                && d.undo()
                && d.snapshot() == old,
            "Alignment context/native/group Undo",
        )?;
    }
    for n in 0..3 {
        let c = Settings {
            selected: settings.selected[..n].to_vec(),
            ..Default::default()
        };
        check(
            a::prepare_advanced(&d, &c, Operation::Edges(Align::Right))? == old,
            "Distribution below three changed context/history",
        )?;
    }
    let mut states = vec![];
    for op in [Operation::GapsHorizontal, Operation::CentersHorizontal] {
        d.commit_state(a::prepare_advanced(&d, &settings, op)?)?;
        states.push(a::bounds(&d, Item::Object(1))?.min[0]);
        d.undo();
    }
    check(
        states == vec![20.0, 23.0],
        "Unequal widths did not distinguish gaps and centers",
    )?;
    d.scene.guides = vec![[[12.5, 0.0], [12.5, 64.0]], [[0.0, 0.0], [64.0, 64.0]]];
    let guide = Settings {
        reference: Reference::Guide(0),
        ..settings.clone()
    };
    d.commit_state(a::prepare_advanced(
        &d,
        &guide,
        Operation::Align(Align::Left),
    )?)?;
    check(
        guide
            .selected
            .iter()
            .all(|i| a::bounds(&d, *i).unwrap().min[0] == 12.5),
        "Guide alignment",
    )?;
    d.undo();
    check(
        a::prepare_advanced(&d, &guide, Operation::Align(Align::Top)).is_err()
            && a::prepare_advanced(
                &d,
                &Settings {
                    reference: Reference::Guide(1),
                    ..settings.clone()
                },
                Operation::Align(Align::Left),
            )
            .is_err(),
        "Wrong-axis/diagonal guide accepted",
    )?;
    check(
        a::prepare_advanced(&d, &Settings::default(), Operation::Align(Align::Left))?
            == d.snapshot(),
        "Empty advanced alignment changed page",
    )?;

    let mut text = make()?;
    text.scene.objects.push(PageObject::Text {
        layout: None,
        baseline: [8.0, 24.0],
        text: "  I ".into(),
        size_px: 16.0,
        color: [20, 40, 200, 255],
    });
    let renderer = crate::render::Renderer::new(&text);
    let occupied = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .filter(|(x, y)| renderer.pixel([*x as f64 + 0.5, *y as f64 + 0.5])[3] > 0)
        .flat_map(|(x, y)| [[x as f64, y as f64], [x as f64 + 1.0, y as f64 + 1.0]]);
    let pixel = detail::rectangle(occupied)?;
    let opts = Options {
        text_pixels: true,
        ..Default::default()
    };
    check(
        detail::bounds(&text, Item::Object(0), opts)? == pixel
            && pixel.min[0] > 8.0
            && pixel.max[0] < a::bounds(&text, Item::Object(0))?.max[0],
        "Text glyph pixels/transparent spaces",
    )?;
    let cfg = Settings {
        selected: vec![Item::Object(0)],
        reference: Reference::Canvas,
        options: opts,
    };
    text.commit_state(a::prepare_advanced(
        &text,
        &cfg,
        Operation::Align(Align::Left),
    )?)?;
    check(
        detail::bounds(&text, Item::Object(0), opts)?.min[0] == 0.0,
        "Text pixel alignment",
    )?;
    text.undo();
    text.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 64.0, 0.0]);
    check(
        detail::bounds(&text, Item::Object(0), opts)?.valid(),
        "Rotated glyph pixel bounds",
    )?;
    if let PageObject::Text { text, .. } = &mut text.scene.objects[0] {
        *text = "  ".into();
    }
    check(
        detail::bounds(&text, Item::Object(0), opts).is_err(),
        "Empty glyph pixels accepted",
    )?;
    if let PageObject::Text { text, size_px, .. } = &mut text.scene.objects[0] {
        *text = "I".into();
        *size_px = 1e6;
    }
    check(
        detail::bounds(&text, Item::Object(0), opts).is_err(),
        "Huge glyph bounds were not bounded",
    )?;
    let duplicate = Settings {
        selected: vec![Item::Object(0), Item::Object(0)],
        ..Default::default()
    };
    check(!duplicate.valid(), "Duplicate remembered object selection")?;
    let mut raster = make()?;
    raster.layers[0].pixels[10 * 64 + 8] = [1, 2, 3, 255];
    let r = detail::bounds(
        &raster,
        Item::Raster(0),
        Options {
            raster_pixels: true,
            ..Default::default()
        },
    )?;
    check(
        r == Rect {
            min: [8.0, 10.0],
            max: [9.0, 11.0],
        },
        "Raster transparent margin bounds",
    )?;

    let mut vector = make()?;
    let layer = vector.scene.lines.add_layer("Control targets")?;
    vector.scene.lines.add_path(
        layer,
        vec![[8.0, 8.0], [24.0, 24.0]],
        false,
        10.0,
        [200, 40, 20, 255],
    )?;
    let id = vector.scene.lines.layers[0].paths[0].id;
    check(
        a::bounds(&vector, Item::Vector(id))?.min == [3.0, 3.0]
            && detail::bounds(
                &vector,
                Item::Vector(id),
                Options {
                    vector_paths: true,
                    ..Default::default()
                },
            )?
            .min == [8.0, 8.0],
        "Vector path/outline bounds",
    )?;
    for (objects, controls, canvas, p, expected) in [
        (true, false, false, [3.5, 12.0], [3.0, 12.0]),
        (false, true, false, [9.0, 9.0], [8.0, 8.0]),
        (false, false, true, [63.0, 30.0], [64.0, 30.0]),
    ] {
        let s = crate::snapping::Settings {
            objects,
            control_points: controls,
            canvas_edges: canvas,
            screen_distance: 2.0,
            ..Default::default()
        };
        let mut runtime = crate::rulers::Runtime::new(&vector)?;
        runtime.configure_snapping(&vector, s, 1.0)?;
        check(
            near(runtime.constrain(p), expected),
            "Object/control/canvas snap target",
        )?;
        runtime.set_snap_disabled(true);
        check(runtime.constrain(p) == p, "Temporary object snap disable")?;
    }
    let s = crate::snapping::Settings {
        control_points: true,
        screen_distance: 2.0,
        ..Default::default()
    };
    for zoom in [0.5, 1.0, 2.0, 4.0] {
        for ppp in [1.0, 2.0] {
            let scale = ppp / zoom;
            let mut r = crate::rulers::Runtime::new(&vector)?;
            r.configure_snapping(&vector, s, scale)?;
            check(
                near(r.constrain([8.0 + 1.999 * scale, 8.0]), [8.0, 8.0])
                    && near(
                        r.constrain([8.0 + 2.001 * scale, 8.0]),
                        [8.0 + 2.001 * scale, 8.0],
                    ),
                "Object screen-distance boundary",
            )?;
        }
    }
    let g = crate::object_snap::Geometry::new(&vector, s, &[Item::Vector(id)])?;
    check(
        g.snap([8.0, 8.0], 2.0).is_none(),
        "Moving object snapped to itself",
    )?;
    vector.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 64.0, 0.0]);
    let g = crate::object_snap::Geometry::new(&vector, s, &[])?;
    check(
        near(
            g.snap([49.0, 16.0], 2.0)
                .ok_or("Transformed control missing")?
                .0,
            [48.0, 16.0],
        ),
        "Transformed control target",
    )?;
    vector.scene.transform = Affine::default();
    vector.scene.lines.layers[0].hidden = true;
    check(
        crate::object_snap::Geometry::new(&vector, s, &[])?
            .snap([8.0, 8.0], 2.0)
            .is_none(),
        "Hidden control target snapped",
    )?;
    let workspace = crate::workspaces::Snapshot {
        snapping: s,
        ..Default::default()
    };
    let file = root.join("object-snap-workspace.json");
    workspace.export(&file).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Snapshot::read(&file)
            .map_err(|e| e.to_string())?
            .snapping
            == s,
        "Object snap settings reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn edge_spacing_text_path_guides_and_object_snap() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
