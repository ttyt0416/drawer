use crate::{
    comics::*,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_search::Key,
    render::Renderer,
    scene::{Affine, PageObject, Rect},
};
fn check(v: bool, s: &str) -> Result<(), String> {
    if v { Ok(()) } else { Err(s.into()) }
}
fn page() -> Document {
    Document::create(
        CanvasSpec {
            width: 32,
            height: 64,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .unwrap()
}
fn rect() -> Rect {
    Rect {
        min: [2., 2.],
        max: [30., 60.],
    }
}
fn add(d: &mut Document, o: Object) -> Result<u64, String> {
    let n = d.scene.objects.len();
    d.commit_state(crate::comics::prepare(d, None, o)?)?;
    Ok(d.scene.catalog.objects.id(n))
}
pub fn objects() -> Result<(), String> {
    let mut d = page();
    for o in defaults(rect()) {
        add(&mut d, o)?;
    }
    check(d.scene.valid(), "Comic scene valid")?;
    let saved = d.snapshot();
    check(
        serde_json::from_slice::<crate::scene::Scene>(&serde_json::to_vec(&d.scene).unwrap())
            .unwrap()
            == d.scene,
        "Comic parameter serialization",
    )?;
    check(
        !Object {
            name: String::new(),
            ..defaults(rect())[0].clone()
        }
        .valid(),
        "Empty name rejected",
    )?;
    let mut o = defaults(rect())[1].clone();
    if let Kind::Frame(f) = &mut o.kind {
        f.polygon.swap(1, 2);
    }
    check(!o.valid(), "Self-intersecting frame rejected")?;
    let mut d = page();
    let s = crate::text_layout::Settings {
        baseline: [4., 22.],
        text: "".into(),
        size: 16.,
        color: [0, 0, 0, 255],
        style: Default::default(),
    };
    let text = crate::text_layout::prepare(&d, None, &s)?;
    let text_id = text.id;
    d.commit_state(text.state)?;
    let mut balloon = defaults(Rect {
        min: [3., 3.],
        max: [15., 25.],
    })[0]
        .clone();
    let Kind::Balloon(b) = &mut balloon.kind else {
        unreachable!()
    };
    b.text = Some(text_id);
    b.tail = Tail::None;
    let empty = balloon.resolved(&d.scene);
    let id = add(&mut d, balloon.clone())?;
    let mut longer = s.clone();
    longer.text = "Long dialogue grows".into();
    d.commit_state(crate::text_layout::prepare(&d, Some(text_id), &longer)?.state)?;
    let grown = balloon.resolved(&d.scene);
    check(
        grown.bounds().max[0] > empty.bounds().max[0],
        "Linked text did not grow balloon",
    )?;
    balloon.transform = Affine([0., 1., -1., 0., 30., 0.]);
    let rotated = balloon.resolved(&d.scene);
    if let Kind::Balloon(b) = &rotated.kind {
        check(b.text == Some(text_id), "Rotation broke stable text link")?;
    }
    d.commit_state(crate::comics::prepare(&d, Some(id), balloon)?)?;
    d.undo();
    d.redo();
    check(
        d.scene.valid() && saved.scene.objects.len() == 4,
        "Comic Undo/Redo",
    )?;
    let mut shapes = std::collections::BTreeSet::new();
    for shape in [
        BalloonShape::Ellipse,
        BalloonShape::Rectangle,
        BalloonShape::Cloud,
    ] {
        for tail in [Tail::Speech, Tail::Thought, Tail::None] {
            let mut o = defaults(rect())[0].clone();
            if let Kind::Balloon(b) = &mut o.kind {
                b.shape = shape;
                b.tail = tail;
                b.tip = [16., 58.];
            }
            let g = Geometry::new(&o, &page().scene);
            let pixels: Vec<_> = (0..64)
                .flat_map(|y| (0..32).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
                .map(|p| g.pixel(p, Affine::default(), 300.))
                .collect();
            check(pixels.iter().any(|p| p[3] > 0), "Empty balloon render")?;
            shapes.insert(pixels);
        }
    }
    check(shapes.len() >= 6, "Balloon shapes/tails all identical")
}
pub fn frames() -> Result<(), String> {
    let mut d = page();
    d.layers[0].pixels.fill([12, 80, 200, 255]);
    let source = Key::Layer(crate::layer_properties::raster(&d, 0));
    let mut frame = defaults(rect())[1].clone();
    if let Kind::Frame(f) = &mut frame.kind {
        f.children.push(source);
        f.width = 0.;
    }
    let id = add(&mut d, frame)?;
    let r = Renderer::new(&d);
    check(
        r.pixel([1.5, 10.5])[3] == 0 && r.pixel([10.5, 10.5]) == [12, 80, 200, 255],
        "Frame contents not clipped",
    )?;
    drop(r);
    let moved = crate::search_move::prepare(&d, Key::Object(id), [1, 1])?;
    check(
        moved.layers[0].extent(d.width, d.height).offset == [1, 1],
        "Frame folder movement left its contents behind",
    )?;
    let same_edge =
        crate::comic_split::prepare(&d, id, &[[10., -5.], [10., 20.], [20., 20.], [20., -5.]])?;
    check(
        same_edge.scene.objects.len() == 2,
        "Same-edge polyline cut failed",
    )?;
    let before = d.snapshot();
    check(
        crate::comic_split::prepare(&d, id, &[[-10., -10.], [-5., -5.]]).is_err(),
        "Outside cut accepted",
    )?;
    let state = crate::comic_split::prepare(&d, id, &[[-5., 32.], [16., 28.], [40., 32.]])?;
    d.commit_state(state)?;
    let renderer = Renderer::new(&d);
    let original = {
        let mut copy = d.clone();
        copy.undo();
        copy
    };
    let a = Renderer::new(&original);
    for y in 0..64 {
        for x in 0..32 {
            check(
                renderer.pixel([x as f64 + 0.5, y as f64 + 0.5])
                    == a.pixel([x as f64 + 0.5, y as f64 + 0.5]),
                "Zero-gap polyline split lost contents",
            )?;
        }
    }
    drop(renderer);
    d.undo();
    check(
        d.snapshot() == before,
        "Frame split Undo did not restore folders/masks",
    )?;
    d.redo();
    for object in &mut d.scene.objects {
        if let PageObject::Comic(c) = object
            && let Kind::Frame(f) = &mut c.kind
        {
            f.gap = 6.;
        }
    }
    check(
        Renderer::new(&d).pixel([16.5, 28.5])[3] == 0,
        "Frame gap not masked",
    )?;
    let mut bad = defaults(rect())[1].clone();
    if let Kind::Frame(f) = &mut bad.kind {
        f.children.push(Key::Object(id));
    }
    let n = d
        .scene
        .catalog
        .objects
        .index(id, d.scene.objects.len())
        .unwrap();
    let mut state = d.snapshot();
    state.scene.objects[n] = PageObject::Comic(Box::new(bad));
    check(!state.scene.valid(), "Frame cycle accepted")
}
pub fn tones() -> Result<(), String> {
    let d = page();
    let mut variants = std::collections::BTreeSet::new();
    for dot in Dot::ALL {
        let mut o = defaults(rect())[2].clone();
        let Kind::Tone(t) = &mut o.kind else {
            unreachable!()
        };
        t.dot = dot;
        t.lpi = 30.;
        t.angle = 23.;
        t.phase = [1.25, -2.];
        t.color[3] = 137;
        let g = Geometry::new(&o, &d.scene);
        let pixels: Vec<_> = (0..64)
            .flat_map(|y| (0..32).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
            .map(|p| g.pixel(p, Affine::default(), 300.))
            .collect();
        check(
            pixels.iter().all(|p| p[3] == 0 || p[3] == 137),
            "Tone alpha changed",
        )?;
        check(pixels.iter().any(|p| p[3] > 0), "Empty tone")?;
        variants.insert(pixels.clone());
        check(
            pixels
                == (0..64)
                    .flat_map(|y| (0..32).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
                    .map(|p| Geometry::new(&o, &d.scene).pixel(p, Affine::default(), 300.))
                    .collect::<Vec<_>>(),
            "Tone seed unstable",
        )?;
        for density in [0., 1.] {
            let mut edge = o.clone();
            if let Kind::Tone(t) = &mut edge.kind {
                t.density = density;
            }
            let g = Geometry::new(&edge, &d.scene);
            for p in [[4.5, 4.5], [18.5, 33.5]] {
                check(
                    g.pixel(p, Affine::default(), 300.)[3] == if density == 0. { 0 } else { 137 },
                    "Tone 0/100 density",
                )?;
            }
        }
        let mut white = o.clone();
        if let Kind::Tone(t) = &mut white.kind {
            t.color = [255, 255, 255, 137];
            t.density = 1.;
        }
        check(
            Geometry::new(&white, &d.scene).pixel([10.5, 10.5], Affine::default(), 300.)[3] == 0,
            "White luminance became ink",
        )?;
        let mut gradient = o.clone();
        if let Kind::Tone(t) = &mut gradient.kind {
            t.gradient = true;
            t.density = 0.;
            t.end_density = 1.;
        }
        let g = Geometry::new(&gradient, &d.scene);
        let band = |range: std::ops::Range<u32>| {
            range
                .flat_map(|y| (3..29).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
                .map(|p| u32::from(g.pixel(p, Affine::default(), 300.)[3]))
                .sum::<u32>()
        };
        check(
            band(48..58) > band(4..14) * 3,
            "Tone gradient density did not grow",
        )?;
        let mut scaled = o.clone();
        scaled.transform = Affine([2., 0., 0., 2., 0., 0.]);
        check(
            Geometry::new(&o, &d.scene).pixel([7.5, 9.5], Affine::default(), 300.)
                == Geometry::new(&scaled, &d.scene).pixel([15., 19.], Affine::default(), 600.),
            "Tone physical frequency changed with DPI",
        )?;
    }
    check(variants.len() == 7, "Tone geometries not distinct")?;
    let mut d = page();
    let mut tone = defaults(rect())[2].clone();
    if let Kind::Tone(t) = &mut tone.kind {
        t.lpi = 150.;
        t.density = 0.5;
    }
    add(&mut d, tone)?;
    let (_, _, p) = Renderer::new(&d).output(75., 1 << 20)?;
    check(
        p.iter().any(|p| p[3] > 0 && p[3] < 255),
        "Reduced tone output missing moiré box preview",
    )
}
pub fn effects() -> Result<(), String> {
    let d = page();
    for count in [1, 2048] {
        for kind in [Lines::Parallel, Lines::Radial] {
            let mut o = defaults(rect())[3].clone();
            if let Kind::Effect(e) = &mut o.kind {
                e.count = count;
                e.kind = kind;
                e.center = [16., 32.];
                e.inner = 0.;
            }
            check(o.valid(), "Effect min/max count rejected")?;
            let a = Geometry::new(&o, &d.scene);
            let b = Geometry::new(&o, &d.scene);
            for y in 0..64 {
                for x in 0..32 {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    check(
                        a.pixel(p, Affine::default(), 300.) == b.pixel(p, Affine::default(), 300.),
                        "Effect fixed seed changed",
                    )?;
                }
            }
        }
    }
    check(d.scene.rulers.active.is_none(), "Effect enabled ruler snap")
}
pub fn bands() -> Result<(), String> {
    let mut d = page();
    for y in 0..64 {
        for x in 0..32 {
            d.layers[0].pixels[y * 32 + x] = [x as u8, y as u8, 100, 255];
        }
    }
    let text = crate::text_layout::Settings {
        baseline: [3., 34.],
        text: "Split".into(),
        size: 16.,
        color: [255, 0, 0, 255],
        style: Default::default(),
    };
    d.commit_state(crate::text_layout::prepare(&d, None, &text)?.state)?;
    let original = d.snapshot();
    let raw = Renderer::new(&d);
    let pixels: Vec<_> = (0..64)
        .flat_map(|y| (0..32).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
        .map(|p| raw.pixel(p))
        .collect();
    drop(raw);
    let insert = crate::webtoon_band::Band::Insert { at: 32, height: 9 };
    check(
        !crate::webtoon_band::impact(&d, insert)?.is_empty(),
        "Missing band impact list",
    )?;
    d.commit_state(crate::webtoon_band::prepare(&d, insert, false)?)?;
    check(d.height == 73, "Band height not changed")?;
    let r = Renderer::new(&d);
    for y in 0..73 {
        for x in 0..32 {
            let expected = if (32..41).contains(&y) {
                [0; 4]
            } else {
                pixels[(if y < 32 { y } else { y - 9 }) * 32 + x]
            };
            check(
                r.pixel([x as f64 + 0.5, y as f64 + 0.5]) == expected,
                "Band insertion lost crossing text/raster pixels",
            )?;
        }
    }
    drop(r);
    check(
        d.scene
            .objects
            .iter()
            .filter(|o| matches!(o, PageObject::Text { .. }))
            .count()
            == 2,
        "Crossing text flattened",
    )?;
    d.undo();
    check(d.snapshot() == original, "Band Undo failed")?;
    check(
        crate::webtoon_band::prepare(
            &d,
            crate::webtoon_band::Band::Remove { start: 32, end: 40 },
            false,
        )
        .is_err(),
        "Unconfirmed strip removal accepted",
    )?;
    let remove = crate::webtoon_band::prepare(
        &d,
        crate::webtoon_band::Band::Remove { start: 32, end: 40 },
        true,
    )?;
    d.commit_state(remove)?;
    let r = Renderer::new(&d);
    for y in 0..56 {
        for x in 0..32 {
            check(
                r.pixel([x as f64 + 0.5, y as f64 + 0.5])
                    == pixels[(if y < 32 { y } else { y + 8 }) * 32 + x],
                "Band removal lost retained rows",
            )?;
        }
    }
    drop(r);
    d.undo();
    for b in [
        crate::webtoon_band::Band::Insert { at: 0, height: 0 },
        crate::webtoon_band::Band::Insert { at: 65, height: 2 },
        crate::webtoon_band::Band::Insert {
            at: 64,
            height: 32768,
        },
        crate::webtoon_band::Band::Remove { start: 0, end: 64 },
    ] {
        check(
            crate::webtoon_band::prepare(&d, b, true).is_err(),
            "Invalid band accepted",
        )?;
    }
    for at in [0, 64] {
        let state = crate::webtoon_band::prepare(
            &d,
            crate::webtoon_band::Band::Insert { at, height: 1 },
            false,
        )?;
        let mut copy = d.clone();
        copy.commit_state(state)?;
        let r = Renderer::new(&copy);
        for y in 0..65 {
            for x in 0..32 {
                let expected = if y == at as usize {
                    [0; 4]
                } else {
                    pixels[(if y < at as usize { y } else { y - 1 }) * 32 + x]
                };
                check(
                    r.pixel([x as f64 + 0.5, y as f64 + 0.5]) == expected,
                    "Band at beginning/end lost pixels",
                )?;
            }
        }
    }
    for (start, end) in [(0, 1), (63, 64)] {
        let state = crate::webtoon_band::prepare(
            &d,
            crate::webtoon_band::Band::Remove { start, end },
            true,
        )?;
        let mut copy = d.clone();
        copy.commit_state(state)?;
        let r = Renderer::new(&copy);
        for y in 0..63 {
            for x in 0..32 {
                let source = if y < start as usize { y } else { y + 1 };
                check(
                    r.pixel([x as f64 + 0.5, y as f64 + 0.5]) == pixels[source * 32 + x],
                    "First/last strip removal lost retained rows",
                )?;
            }
        }
    }
    d.layers[0].locked = true;
    check(
        crate::webtoon_band::prepare(&d, insert, true).is_err(),
        "Locked layer moved by band",
    )
}
pub fn rich_bands() -> Result<(), String> {
    rich_bands_fixture(false)
}
pub fn rich_frame_bands() -> Result<(), String> {
    rich_bands_fixture(true)
}
fn rich_bands_fixture(clipped: bool) -> Result<(), String> {
    let mut d = page();
    d.color = Default::default();
    d.layers[0].precise = Some(vec![[12345, 23456, 34567, 65535]; 32 * 64]);
    d.layers[0].pixels =
        vec![crate::color_precision::Sample::from16([12345, 23456, 34567, 65535]).rgba8(); 32 * 64];
    d.scene.lines.next_id = 3;
    d.scene.lines.layers.push(crate::line_tools::Layer {
        fill: Default::default(),
        id: 1,
        name: "crossing vector".into(),
        hidden: false,
        locked: false,
        paths: vec![crate::line_tools::Path {
            svg: None,
            tip: Default::default(),
            pressure: Default::default(),
            fill: None,
            stroke: true,
            definition: None,
            id: 2,
            points: vec![[4., 10.], [24., 54.]],
            closed: false,
            width: 3.,
            color: [240, 0, 30, 255],
        }],
    });
    d.scene.camera = Some(crate::scene::Camera {
        page_bounds: Rect {
            min: [0.; 2],
            max: [32., 64.],
        },
        position: [0.; 3],
        pixels_per_unit: 1.,
    });
    d.scene.worlds.push(crate::scene::WorldMesh {
        vertices: vec![[-10., -30., 0.], [10., 30., 0.]],
        edges: vec![[0, 1]],
        color: [40, 200, 80, 255],
    });
    d.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 2,
        tracks: vec![crate::animation::Track {
            name: "timing".into(),
            cells: vec![crate::animation::Cell {
                name: "cell".into(),
                width: 2,
                height: 64,
                offset: [2, 0],
                pixels: std::sync::Arc::new(vec![[0, 0, 255, 200]; 128]),
            }],
            exposures: vec![crate::animation::Exposure { frame: 0, cell: 0 }],
            end: 2,
        }],
        audio: vec![],
    });
    check(
        d.scene.valid() && d.color.valid_layers(&d.layers),
        "Rich band fixture invalid",
    )?;
    if clipped {
        let mut frame = defaults(rect())[1].clone();
        let Kind::Frame(f) = &mut frame.kind else {
            unreachable!()
        };
        f.width = 0.;
        f.children = vec![
            Key::Layer(crate::layer_properties::raster(&d, 0)),
            Key::Layer(crate::layer_properties::Target::Vector(1)),
            Key::World(1),
            Key::Track(1),
        ];
        add(&mut d, frame)?;
    }
    let before = d.snapshot();
    let original = Renderer::new(&d);
    let pixels: Vec<_> = (0..64)
        .flat_map(|y| (0..32).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
        .map(|p| original.pixel(p))
        .collect();
    drop(original);
    d.commit_state(crate::webtoon_band::prepare(
        &d,
        crate::webtoon_band::Band::Insert { at: 32, height: 8 },
        false,
    )?)?;
    check(
        d.layers.iter().all(|l| {
            l.precise
                .as_ref()
                .is_some_and(|p| p.iter().all(|v| *v == [12345, 23456, 34567, 65535]))
        }),
        "Band flattened precision raster",
    )?;
    check(
        d.scene.lines.layers.len() == 2 && d.scene.worlds.len() == 2,
        "Band flattened vector/3D source",
    )?;
    check(
        d.scene.camera == before.scene.camera
            && d.animation.as_ref().unwrap().tracks[0].exposures
                == before.animation.as_ref().unwrap().tracks[0].exposures,
        "Band changed camera/timing",
    )?;
    let r = Renderer::new(&d);
    for y in 0..72 {
        for x in 0..32 {
            let expected = if (32..40).contains(&y) {
                [0; 4]
            } else {
                pixels[(if y < 32 { y } else { y - 8 }) * 32 + x]
            };
            check(
                r.pixel([x as f64 + 0.5, y as f64 + 0.5]) == expected,
                "Rich band changed retained vector/camera/animation rows",
            )?;
        }
    }
    drop(r);
    d.undo();
    check(d.snapshot() == before, "Rich band Undo")
}
pub fn dialogue() -> Result<(), String> {
    let mut docs = vec![page(), page()];
    let s = crate::text_layout::Settings {
        baseline: [2., 20.],
        text: "same\nname".into(),
        size: 12.,
        color: [0, 0, 0, 255],
        style: Default::default(),
    };
    for d in &mut docs {
        d.commit_state(crate::text_layout::prepare(d, None, &s)?.state)?;
    }
    let before: Vec<_> = docs.iter().map(Document::snapshot).collect();
    let mut rows = crate::comic_dialogue::rows(&docs.iter().collect::<Vec<_>>());
    for row in &mut rows {
        row.after.text.push_str(" edited");
    }
    let prepared = crate::comic_dialogue::prepare(&docs.iter().collect::<Vec<_>>(), &rows)?;
    check(
        docs.iter().zip(&before).all(|(d, s)| d.snapshot() == *s),
        "Dialogue preview mutated source",
    )?;
    let t = prepared.apply(&mut docs)?;
    check(
        docs.iter().all(
            |d| matches!(&d.scene.objects[0],PageObject::Text{text,..}if text.ends_with(" edited")),
        ),
        "Dialogue changed wrong duplicate name",
    )?;
    t.step(&mut docs, false)?;
    check(
        docs.iter().zip(&before).all(|(d, s)| d.snapshot() == *s),
        "Linked dialogue Undo failed",
    )?;
    t.step(&mut docs, true)?;
    check(
        crate::comic_dialogue::prepare(&docs.iter().collect::<Vec<_>>(), &rows).is_err(),
        "Stale dialogue row accepted",
    )?;
    check(
        crate::comic_dialogue::insert(
            &docs[0],
            None,
            &s,
            "x",
            crate::comic_dialogue::Insertion::Current,
            true,
        )
        .is_err(),
        "Missing inheritance source accepted",
    )?;
    let id = docs[0].scene.catalog.objects.id(0);
    let edit = crate::comic_dialogue::insert(
        &docs[0],
        Some(id),
        &s,
        "\nnew",
        crate::comic_dialogue::Insertion::Current,
        true,
    )?;
    check(
        edit.state.scene.objects.len() == 1
            && matches!(&edit.state.scene.objects[0],PageObject::Text{text,..}if text.ends_with("\nnew")),
        "Dialogue append did not preserve original",
    )
}
pub fn metadata() -> Result<(), String> {
    let d = page();
    let mut p = PageInfo::default();
    for first in [false, true] {
        for right in [false, true] {
            p.first_right = first;
            p.right_binding = right;
            p.total = 5;
            for n in 1..=5 {
                p.number = n;
                check(
                    p.valid() && p.facing().is_none_or(|f| (1..=5).contains(&f) && f != n),
                    "Facing page range/odd side",
                )?;
            }
        }
    }
    p.number = 6;
    check(!p.valid(), "Out-of-range page number accepted")?;
    p.number = 1;
    p.bleed_mm = 0.;
    check(p.guides(&d).is_ok(), "Zero bleed rejected")?;
    p.bleed_mm = 10.;
    check(p.guides(&d).is_err(), "Oversized bleed accepted")?;
    let v = crate::comic_view::Viewport {
        enabled: true,
        width: 800,
        height: 1280,
    };
    for top in [-100., 0., 10000.] {
        let r = v.region(32, 64, top);
        check(
            r.min[1] >= 0. && r.max[1] <= 64.,
            "Viewport end outside document",
        )?;
    }
    check(
        v.region(32, 4, 1.).max[1] == 4.,
        "Short viewport not clipped",
    )
}
pub fn metadata_output() -> Result<(), String> {
    let mut d = page();
    d.dpi = 254.;
    let mut info = PageInfo {
        title: "Local work".into(),
        author: "Author".into(),
        trim_mm: [2.8, 6.0],
        bleed_mm: 0.2,
        safe_mm: 0.1,
        number: 3,
        total: 5,
        include_guides: true,
        ..Default::default()
    };
    info.guides(&d)?;
    let mut state = d.snapshot();
    state.scene.comics.page = info.clone();
    d.commit_state(state)?;
    let before = d.snapshot();
    let guide = Renderer::for_export(&d)?;
    check(
        guide.pixel([2.5, 10.5])[3] > 0,
        "Integer-aligned trim guide disappeared from output",
    )?;
    check(
        Renderer::new(&d).pixel([2.5, 10.5]) == [0; 4],
        "Output guide changed document pixels",
    )?;
    let (_, _, pixels) = guide.output(508., 1 << 20)?;
    check(
        pixels.iter().any(|p| p[3] > 0),
        "Guide missing from resized output",
    )?;
    drop(guide);
    info.include_guides = false;
    info.include_number = true;
    d.scene.comics.page = info.clone();
    let number = Renderer::for_export(&d)?;
    check(
        (0..64).any(|y| (0..32).any(|x| number.pixel([x as f64 + 0.5, y as f64 + 0.5])[3] > 0)),
        "Selected page number absent from output",
    )?;
    drop(number);
    check(
        Renderer::new(&d).pixel([16.5, 58.5]) == [0; 4],
        "Page number changed view pixels",
    )?;
    let saved =
        serde_json::from_slice::<crate::scene::Scene>(&serde_json::to_vec(&d.scene).unwrap())
            .map_err(|e| e.to_string())?;
    check(
        saved.comics.page == info,
        "Page metadata/output flags serialization",
    )?;
    info.include_number = false;
    d.scene.comics.page = info;
    let plain = Renderer::for_export(&d)?;
    check(
        (0..64).all(|y| (0..32).all(|x| plain.pixel([x as f64 + 0.5, y as f64 + 0.5]) == [0; 4])),
        "Output exclusion flags ignored",
    )?;
    drop(plain);
    d.scene.comics.page = before.scene.comics.page.clone();
    check(
        d.snapshot() == before,
        "Metadata output mutated the document",
    )?;
    d.undo();
    check(
        d.scene.comics.page == PageInfo::default(),
        "Page information not one Undo",
    )
}
pub fn metadata_files(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("metadata-float-output");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 32,
                height: 64,
                dpi: 254.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            Default::default(),
        )
        .map_err(|e| e.to_string())?;
        let sample = crate::color_precision::Sample::from16([12345, 23456, 34567, 65535]);
        d.layers[0].precise.as_mut().unwrap()[32 * 32 + 16] = sample.rgba16();
        d.layers[0].pixels[32 * 32 + 16] = sample.rgba8();
        d.scene.comics.page = PageInfo {
            trim_mm: [2.8, 6.],
            bleed_mm: 0.2,
            safe_mm: 0.1,
            include_guides: true,
            ..Default::default()
        };
        let before = d.snapshot();
        let path = dir.join("guides.png");
        crate::png_export::export(&d, &path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        let image = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
        check(
            image.get_pixel(2, 10).0[3] > 0,
            "Float-color PNG omitted selected print guides",
        )?;
        check(
            image.get_pixel(16, 32).0 == sample.rgba8() && d.snapshot() == before,
            "Guide output changed color samples or original sixteen-bit data",
        )?;
        d.scene.comics.page.include_guides = false;
        let path = dir.join("plain.png");
        crate::png_export::export(&d, &path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(
            image::open(&path)
                .map_err(|e| e.to_string())?
                .to_rgba8()
                .get_pixel(2, 10)
                .0[3]
                == 0,
            "Float PNG ignored output-guide exclusion",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(dir).map_err(|e| e.to_string());
    result.and(cleanup)
}
pub fn files(root: &std::path::Path) -> Result<(), String> {
    let root = root.join("comic-fixture");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = page();
        for o in defaults(rect()) {
            add(&mut d, o)?;
        }
        let storage = crate::storage::UserStorage::at(root.join("local-settings"))
            .map_err(|e| e.to_string())?;
        let mut store = crate::workspaces::Store::default();
        store.current.comic_view.show_breaks = true;
        store.current.comic_view.vertical_list = true;
        store.current.comic_view.set(
            root.join("comic.drawer"),
            crate::comic_view::Viewport {
                enabled: true,
                width: 390,
                height: 844,
            },
        );
        store.save(&storage).map_err(|e| e.to_string())?;
        let restored = crate::workspaces::Store::load(&storage)
            .map_err(|e| e.to_string())?
            .ok_or("Saved workspace missing")?;
        check(
            restored.current.comic_view == store.current.comic_view,
            "Comic local view preferences not restored",
        )?;
        let path = root.join("comic.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let mut old = std::fs::read(&path).map_err(|e| e.to_string())?;
        old[8..12].copy_from_slice(&92u32.to_le_bytes());
        let downgraded = root.join("old-header.drawer");
        std::fs::write(&downgraded, old).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&downgraded, DEFAULT_DOCUMENT_BUDGET).is_err(),
            "Old native header accepted comic fields",
        )?;
        let saved = d.snapshot();
        let copy = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(copy.snapshot() == saved, "Comic native roundtrip")?;
        let settings = Webtoon {
            slice: 20,
            cuts: vec![11, 40],
            prefix: "strip".into(),
        };
        let plan = crate::webtoon::plan(d.height, &settings)?;
        check(
            plan.first().unwrap().start == 0
                && plan.last().unwrap().end == 64
                && plan.last().unwrap().end - plan.last().unwrap().start == 4
                && plan.windows(2).all(|r| r[0].end == r[1].start),
            "Split has missing/duplicated rows",
        )?;
        let before = d.snapshot();
        let paths = crate::webtoon::export(&d, &settings, &root, &mut |_, _| Ok(()))?;
        let mut joined = vec![];
        for p in &paths {
            joined.extend(
                image::open(p)
                    .map_err(|e| e.to_string())?
                    .to_rgba8()
                    .pixels()
                    .map(|p| p.0),
            );
        }
        let r = Renderer::for_export(&d)?;
        check(
            joined
                == (0..64)
                    .flat_map(|y| (0..32).map(move |x| [x as f64 + 0.5, y as f64 + 0.5]))
                    .map(|p| r.pixel(p))
                    .collect::<Vec<_>>(),
            "Webtoon strips differ from complete text/pixel render",
        )?;
        check(d.snapshot() == before, "Webtoon export changed source")?;
        check(
            crate::webtoon::export(&d, &settings, &root, &mut |_, _| Ok(())).is_err(),
            "Output collision overwritten",
        )?;
        let mut invalid = d.clone();
        invalid.scene.transform = Affine([0.; 6]);
        let invalid_settings = Webtoon {
            prefix: "invalid".into(),
            ..settings.clone()
        };
        check(
            crate::webtoon::export(&invalid, &invalid_settings, &root, &mut |_, _| Ok(())).is_err(),
            "Invalid scene produced transparent webtoon files",
        )?;
        let race = Webtoon {
            prefix: "race".into(),
            ..settings.clone()
        };
        let race_plan = crate::webtoon::plan(d.height, &race)?;
        let foreign = root.join(&race_plan[1].name);
        let mut injected = false;
        let collision = crate::webtoon::export(&d, &race, &root, &mut |done, _| {
            if !injected && done == d.height as usize + 1 {
                std::fs::write(&foreign, b"foreign file").map_err(|e| e.to_string())?;
                injected = true;
            }
            Ok(())
        });
        check(
            collision.is_err()
                && std::fs::read(&foreign).map_err(|e| e.to_string())? == b"foreign file"
                && !root.join(&race_plan[0].name).exists(),
            "Publication race overwrote a file or retained a partial result",
        )?;
        let cancel = Webtoon {
            prefix: "cancel".into(),
            ..settings.clone()
        };
        check(
            crate::webtoon::export(&d, &cancel, &root, &mut |done, total| {
                if done >= total - 1 {
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            })
            .is_err(),
            "Output cancellation ignored",
        )?;
        check(
            !std::fs::read_dir(&root).unwrap().any(|e| {
                let n = e.unwrap().file_name().to_string_lossy().into_owned();
                n.starts_with("cancel") || n.starts_with(".drawer-strips")
            }),
            "Cancelled output left files/staging",
        )?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(&root).map_err(|e| e.to_string());
    result.and(cleanup)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    objects()?;
    frames()?;
    tones()?;
    effects()?;
    bands()?;
    rich_bands()?;
    rich_frame_bands()?;
    dialogue()?;
    metadata()?;
    metadata_output()?;
    metadata_files(root)?;
    files(root)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comic_object_parameters_and_linked_balloon() {
        objects().unwrap();
    }
    #[test]
    fn frame_polyline_cut_masks_and_undo() {
        frames().unwrap();
    }
    #[test]
    fn seven_tone_geometries_frequency_density_alpha_and_reduced_output() {
        tones().unwrap();
    }
    #[test]
    fn deterministic_effect_lines_min_max_and_center() {
        effects().unwrap();
    }
    #[test]
    fn editable_webtoon_insert_remove_masks_and_atomic_undo() {
        bands().unwrap();
    }
    #[test]
    fn bands_preserve_vector_camera_timeline_and_sixteen_bit_buffers() {
        rich_bands().unwrap();
    }
    #[test]
    fn band_inside_frame_keeps_shared_animation_folder_clip() {
        rich_frame_bands().unwrap();
    }
    #[test]
    fn ordered_dialogue_duplicate_names_stale_rows_and_linked_undo() {
        dialogue().unwrap();
    }
    #[test]
    fn page_metadata_facing_sides_and_viewport() {
        metadata().unwrap();
    }
    #[test]
    fn page_trim_guides_numbers_output_exclusion_dpi_and_document_undo() {
        metadata_output().unwrap();
    }
    #[test]
    fn floating_color_png_guides_respect_flags_without_mutating_sixteen_bits() {
        let root = crate::test_support::TempDir::new();
        metadata_files(&root.0).unwrap();
    }
    #[test]
    fn comic_native_and_exact_strip_output_collision_and_cancel() {
        let root = std::env::temp_dir().join(format!("drawer-comic-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        files(&root).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
}
