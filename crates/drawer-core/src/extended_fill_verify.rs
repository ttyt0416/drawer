use crate::{
    bucket::{self, Region, Settings as Fill},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    extended_fill::{Boundary, Settings, Target},
    scene::Rect,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make(w: u32, h: u32) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn count(r: &Region) -> usize {
    (0..r.height)
        .map(|y| (0..r.width).filter(|x| r.coverage(*x, y) > 0).count())
        .sum()
}
fn animation(doc: &mut Document) {
    doc.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 4,
        tracks: vec![],
        audio: vec![],
    });
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make(11, 11)?;
    for y in 1..10 {
        for x in 1..10 {
            d.layers[0].pixels[y * 11 + x] = if x == 1 || x == 9 || y == 1 || y == 9 {
                [0, 0, 0, 255]
            } else {
                [255; 4]
            };
        }
    }
    for (x, p) in [
        (3, [0; 4]),
        (4, [255, 0, 0, 255]),
        (5, [0, 0, 0, 255]),
        (6, [255, 255, 255, 128]),
        (7, [255, 0, 0, 128]),
    ] {
        d.layers[0].pixels[3 * 11 + x] = p;
    }
    let before = d.snapshot();
    let cases = [
        (Target::MatchSeed, 45, 44),
        (Target::AllColors, 121, 121),
        (Target::Transparent, 41, 43),
        (Target::OtherThanTransparent, 80, 78),
        (Target::SurroundedByTransparent, 80, 78),
        (Target::Black, 33, 33),
        (Target::OtherThanBlack, 88, 88),
        (Target::SurroundedByBlack, 48, 48),
        (Target::WhiteAndTransparent, 86, 87),
        (Target::OtherThanWhiteAndTransparent, 35, 34),
        (Target::SurroundedByWhiteAndTransparent, 35, 34),
        (Target::AllEnclosedExceptTransparency, 47, 45),
        (Target::AllEnclosedIncludingTransparency, 48, 48),
    ];
    for (target, normal, semi) in cases {
        for semi_transparent in [false, true] {
            let s = Fill {
                antialias: false,
                advanced: Settings {
                    target,
                    semi_transparent,
                    connected: false,
                    ..Default::default()
                },
                ..Default::default()
            };
            let r = Region::build(&d, 0, [2.5, 2.5], s)?;
            let wanted = if semi_transparent { semi } else { normal };
            check(
                count(&r) == wanted,
                &format!(
                    "Target {target:?} semi={semi_transparent}: {} instead of {wanted}",
                    count(&r)
                ),
            )?;
        }
    }
    check(d.snapshot() == before, "Target discovery edited source")?;
    let mut diagonal = make(3, 3)?;
    diagonal.layers[0].pixels[0] = [255, 0, 0, 255];
    diagonal.layers[0].pixels[4] = [255, 0, 0, 255];
    for connected in [false, true] {
        let s = Fill {
            antialias: false,
            advanced: Settings {
                target: Target::OtherThanTransparent,
                connected,
                ..Default::default()
            },
            ..Default::default()
        };
        check(
            count(&Region::build(&diagonal, 0, [0.5, 0.5], s)?) == if connected { 1 } else { 2 },
            "Four-way / separated fill connectivity",
        )?;
    }
    let mut corners = make(7, 7)?;
    for v in 1..6 {
        for (x, y) in [(v, 1), (v, 5), (1, v), (5, v)] {
            corners.layers[0].pixels[y * 7 + x] = [0, 0, 0, 255];
        }
    }
    corners.layers[0].pixels[8] = [0; 4];
    let enclosed = Fill {
        antialias: false,
        advanced: Settings {
            target: Target::AllEnclosedIncludingTransparency,
            connected: false,
            ..Default::default()
        },
        ..Default::default()
    };
    check(
        count(&Region::build(&corners, 0, [3.5, 3.5], enclosed)?) == 9,
        "Diagonal corner leaked enclosed fill",
    )?;
    corners.layers[0].pixels[9] = [0; 4];
    check(
        count(&Region::build(&corners, 0, [3.5, 3.5], enclosed)?) == 0,
        "Open side accepted as enclosed",
    )?;
    let mut frame_doc = make(7, 7)?;
    animation(&mut frame_doc);
    for x in 1..7 {
        frame_doc.layers[0].pixels[3 * 7 + x] = [0, 0, 0, 255];
    }
    let bounds = Rect {
        min: [1.0, 1.0],
        max: [6.0, 6.0],
    };
    let before = frame_doc.snapshot();
    frame_doc.edit_scene(crate::animation_output::prepare(&frame_doc, bounds)?)?;
    check(
        frame_doc.history_counts().0 == 1
            && frame_doc.undo()
            && frame_doc.snapshot() == before
            && frame_doc.redo(),
        "Output frame independent Undo/Redo",
    )?;
    let canvas = Fill {
        antialias: false,
        ..Default::default()
    };
    let framed = Fill {
        advanced: Settings {
            boundary: Boundary::AnimationFrame,
            ..Default::default()
        },
        ..canvas
    };
    let a = Region::build(&frame_doc, 0, [2.5, 2.5], canvas)?;
    let b = Region::build(&frame_doc, 0, [2.5, 2.5], framed)?;
    check(
        a.coverage(2, 5) > 0 && count(&b) == 10 && b.coverage(2, 5) == 0 && b.coverage(0, 0) == 0,
        "Open line outside frame bypassed closed output boundary",
    )?;
    check(
        Region::build(&frame_doc, 0, [0.5, 0.5], framed).is_err(),
        "Seed outside frame accepted",
    )?;
    frame_doc.current_frame = 2;
    frame_doc.edit_scene(crate::animation_output::prepare(
        &frame_doc,
        Rect {
            min: [2.0, 1.0],
            max: [5.0, 3.0],
        },
    )?)?;
    frame_doc.current_frame = 1;
    check(
        count(&Region::build(&frame_doc, 0, [2.5, 2.5], framed)?) == 10,
        "Animation output frame hold exposure",
    )?;
    frame_doc.current_frame = 2;
    check(
        count(&Region::build(&frame_doc, 0, [2.5, 2.5], framed)?) == 6,
        "Active animation frame key",
    )?;
    let file = root.join("animation-output-fill.drawer");
    crate::files::save(&mut frame_doc, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == frame_doc.snapshot(),
        "Output frame native reopen",
    )?;
    let key = frame_doc.scene.animation_output.at(2).unwrap();
    frame_doc.scene.transform = crate::scene::Affine([0.0, 2.0, -2.0, 0.0, 7.0, 1.0]);
    let at = frame_doc.scene.transform.map([2.5, 1.5]);
    check(
        crate::extended_fill::inside(&frame_doc, Some(key), at)
            && !crate::extended_fill::inside(
                &frame_doc,
                Some(key),
                frame_doc.scene.transform.map([1.5, 1.5]),
            ),
        "Output frame lost page rotation/scale",
    )?;
    let fresh = make(3, 3)?;
    let before = fresh.snapshot();
    let state = crate::animation_output::prepare_state(
        &fresh,
        Rect {
            min: [0.0, 0.0],
            max: [2.0, 2.0],
        },
        4,
    )?;
    check(
        state
            .animation
            .as_ref()
            .is_some_and(|a| a.frames == 4 && a.valid())
            && state.scene.animation_output.keys.len() == 1
            && fresh.snapshot() == before,
        "Local animation/output frame preparation",
    )?;
    let mut excluded = make(3, 1)?;
    excluded.layers[0].pixels = vec![[10, 20, 30, 128], [11, 20, 30, 255], [10, 20, 30, 0]];
    for tolerance in [0, 1] {
        let s = Fill {
            antialias: false,
            advanced: Settings {
                target: Target::AllColors,
                connected: false,
                excluded_color: Some([10, 20, 30, 255]),
                excluded_tolerance: tolerance,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = Region::build(&excluded, 0, [0.5, 0.5], s)?;
        check(
            r.coverage(0, 0) == 0
                && r.coverage(1, 0) == if tolerance == 0 { 255 } else { 0 }
                && r.coverage(2, 0) == 255,
            "Excluded RGB tolerance / alpha independence / hidden RGB",
        )?;
    }
    let mut line = make(11, 11)?;
    let layer = line.scene.lines.add_layer("Thick boundary")?;
    line.scene.lines.add_path(
        layer,
        vec![[2.5, 2.5], [8.5, 2.5], [8.5, 8.5], [2.5, 8.5]],
        true,
        4.0,
        [0, 0, 0, 255],
    )?;
    let s = Fill {
        reference: bucket::Reference::Composite,
        antialias: false,
        ..Default::default()
    };
    let outer = Region::build(&line, 0, [5.5, 5.5], s)?;
    let center = Region::build(
        &line,
        0,
        [5.5, 5.5],
        Fill {
            advanced: Settings {
                vector_centerline: true,
                ..Default::default()
            },
            ..s
        },
    )?;
    check(
        outer.coverage(4, 5) == 0
            && center.coverage(4, 5) == 255
            && center.coverage(1, 5) == 0
            && crate::render::Renderer::new(&line).pixel([4.5, 5.5]) == [0, 0, 0, 255],
        "Centerline fill changed visible stroke / failed to reach centerline",
    )?;
    let mut ink = make(7, 7)?;
    ink.layers.push(ink.layers[0].clone());
    for v in 1..6 {
        for (x, y) in [(v, 1), (v, 5), (1, v), (5, v)] {
            ink.layers[1].pixels[y * 7 + x] = [0, 0, 0, 255];
        }
    }
    ink.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 0,
            y: 3,
            length: 7,
            alpha: 128,
        }],
    });
    for outlines in [false, true] {
        let before = ink.snapshot();
        let state = bucket::prepare(
            &ink,
            0,
            Some([3.5, 3.5]),
            [200, 40, 20, 255],
            false,
            false,
            Fill {
                reference: bucket::Reference::Layer(1),
                antialias: false,
                advanced: Settings {
                    include_outlines: outlines,
                    outline_distance: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        check(
            state.layers[0].pixels[3 * 7 + 1][3] == if outlines { 128 } else { 0 }
                && state.layers[0].pixels[3 * 7][3] == 0
                && state.layers[0].pixels[2 * 7 + 3][3] == 0
                && state.layers[1] == before.layers[1]
                && state.scene == before.scene,
            "Outline underpaint escaped selection/changed source",
        )?;
        ink.commit_state(state)?;
        check(
            ink.undo() && ink.snapshot() == before,
            "Advanced fill one Undo",
        )?;
    }
    let settings = Fill {
        advanced: Settings {
            target: Target::AllEnclosedExceptTransparency,
            semi_transparent: true,
            connected: false,
            boundary: Boundary::AnimationFrame,
            vector_centerline: true,
            excluded_color: Some([10, 20, 30, 255]),
            excluded_tolerance: 7,
            include_outlines: true,
            outline_distance: 3,
        },
        ..Default::default()
    };
    let mut w = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    w.input.fills[0] = settings;
    let file = root.join("extended-fill.json");
    w.export(&file).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Snapshot::read(&file)
            .map_err(|e| e.to_string())?
            .input
            .fills[0]
            == settings,
        "Advanced fill workspace reopen",
    )?;
    let preset =
        crate::brush_presets::Preset::capture(&w, crate::assets::BrushTip::Pen, "Extended fill")?;
    let preset = crate::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    check(
        preset
            .apply(&crate::workspaces::Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })?
            .input
            .fills[0]
            == settings,
        "Advanced fill preset reopen",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn target_matrix_frame_keys_vectors_exclusions_outlines_storage() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
