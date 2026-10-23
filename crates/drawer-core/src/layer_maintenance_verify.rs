use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_maintenance::{self as core, Operation},
    layer_properties::{self as p, Target},
    layer_search::Key,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for n in 1..9 {
        let mut l = d.layers[0].clone();
        l.name = format!("Raster {}", n + 1);
        d.layers.push(l);
    }
    d.scene.layer_ids.initialize(d.layers.len())?;
    d.layers[0].pixels.fill([20, 40, 200, 128]);
    d.layers[2].pixels.fill([220, 20, 40, 128]);
    d.scene.paper.layers.entry(2).or_default().hidden = true;
    d.scene
        .layer_effects
        .rasters
        .entry(4)
        .or_default()
        .adjustment = Some(Default::default());
    d.scene.paper.layers.entry(4).or_default().fill.reference = true;
    d.layers[5].locked = true;
    d.scene.appearance.set(
        Target::Raster(7),
        crate::appearance::Style {
            enabled: true,
            color: Some([4, 5, 6]),
            ..Default::default()
        },
    )?;
    d.scene.paper.layers.entry(7).or_default().pattern.source = 1;
    d.scene.paper_background = Some(9);
    d.scene.objects.push(crate::scene::PageObject::Text {
        layout: None,
        baseline: [0., 7.],
        text: String::new(),
        size_px: 4.,
        color: [0, 0, 0, 255],
    });
    d.scene.lines.add_layer("Empty vector")?;
    d.animation = Some(crate::animation::Timeline {
        fps: Default::default(),
        frames: 6,
        audio: vec![],
        tracks: vec![crate::animation::Track {
            name: "Cells".into(),
            end: 6,
            cells: (0..3)
                .map(|n| crate::animation::Cell {
                    name: format!("Cell {n}"),
                    width: 1,
                    height: 1,
                    offset: [2, 2],
                    pixels: std::sync::Arc::new(vec![[1, 2, 3, 255]]),
                })
                .collect(),
            exposures: vec![
                crate::animation::Exposure { frame: 0, cell: 0 },
                crate::animation::Exposure { frame: 2, cell: 1 },
                crate::animation::Exposure { frame: 4, cell: 2 },
            ],
        }],
    });
    d.scene.layer_properties.set(
        Target::Cell { track: 0, cell: 1 },
        p::Properties {
            hidden: true,
            ..Default::default()
        },
    );
    d.scene.layer_properties.set(
        Target::Cell { track: 0, cell: 2 },
        p::Properties {
            hidden: true,
            position_locked: true,
            ..Default::default()
        },
    );
    let mut s = d.snapshot();
    crate::layer_catalog::reconcile(&d, &mut s)?;
    s.restore(&mut d);
    let s = crate::layer_comps::prepare(
        &d,
        crate::layer_comps::Command::Capture {
            name: "Before cleanup".into(),
            replace: None,
        },
    )?
    .state;
    s.restore(&mut d);
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    crate::layer_properties_verify::verify(root)?;
    for operation in [Operation::Empty, Operation::Hidden] {
        let mut d = fixture()?;
        let before = d.snapshot();
        let counts = d.history_counts();
        let result = core::prepare(&d, operation)?;
        let candidates = result.candidates.iter().map(|v| v.0).collect::<Vec<_>>();
        let expected = if operation == Operation::Empty {
            vec![
                Key::Layer(Target::Raster(2)),
                Key::Layer(Target::Vector(1)),
                Key::Object(1),
            ]
        } else {
            vec![
                Key::Layer(Target::Raster(3)),
                Key::Cell { track: 1, cell: 2 },
            ]
        };
        check(
            candidates.len() == expected.len() && expected.iter().all(|k| candidates.contains(k)),
            "Cleanup candidates include protected/effect/reference/paper/valid content or omit empties",
        )?;
        check(
            d.snapshot() == before && d.history_counts() == counts,
            "Cleanup preparation changed document/history",
        )?;
        check(
            result.comp_references == vec![("Before cleanup".into(), expected.len())],
            "Cleanup comp impact",
        )?;
        check(
            result.exposures == usize::from(operation == Operation::Hidden),
            "Cleanup exposure impact",
        )?;
        if operation == Operation::Hidden {
            check(
                result
                    .retained
                    .iter()
                    .any(|v| v.0 == Key::Cell { track: 1, cell: 3 }),
                "Position-locked cell not reported retained",
            )?;
        }
        d.commit_state(result.state)?;
        let after = d.snapshot();
        if operation == Operation::Hidden {
            let t = &d.animation.as_ref().unwrap().tracks[0];
            check(
                t.cells.len() == 2
                    && t.exposures
                        == vec![
                            crate::animation::Exposure { frame: 0, cell: 0 },
                            crate::animation::Exposure { frame: 4, cell: 1 },
                        ],
                "Cleanup retargeted exposure or lost retained cell",
            )?;
        }
        check(
            crate::layer_comps::prepare(&d, crate::layer_comps::Command::Apply(1))?
                .missing
                .len()
                == expected.len(),
            "Deleted comp entries not reported missing",
        )?;
        let file = root.join("layer-maintenance.drawer");
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        let reopened =
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(reopened.snapshot() == after, "Cleanup native roundtrip")?;
        check(
            d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
            "Cleanup single Undo/Redo structure restoration",
        )?;
    }
    copy_precision(root)?;
    boundaries()?;
    Ok(())
}
fn copy_precision(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        color_precision::{Depth, Sample},
        color_storage::Config,
    };
    for config in [
        Config::legacy(),
        Config {
            depth: Depth::Eight,
            ..Default::default()
        },
        Config::default(),
    ] {
        let mut d = Document::create_with_color(
            CanvasSpec {
                width: 3,
                height: 2,
                dpi: 300.,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
            config,
        )
        .map_err(|e| e.to_string())?;
        let extent = d.layers[0].extent(3, 2);
        let sample = Sample::from16([12345, 23456, 34567, 30000]);
        let color = d.color.clone();
        crate::color_storage::set_samples(&mut d.layers[0], &color, extent, vec![sample; 6])?;
        d.layers[0].locked = true; // Copy reads locked content without consuming it.
        d.scene.paper.layers.entry(0).or_default().fill.draft = true;
        d.scene.objects.push(crate::scene::PageObject::Line {
            start: [0., 1.],
            end: [3., 1.],
            width_px: 1.,
            color: [240, 40, 20, 128],
        });
        let before = d.snapshot();
        let renderer = crate::render::Renderer::new(&d);
        let expected = (0..6)
            .map(|n| {
                renderer
                    .sample([(n % 3) as f64 + 0.5, (n / 3) as f64 + 0.5])
                    .map(|s| {
                        s.store(
                            d.color.depth,
                            d.color.dither,
                            [(n % 3) as i64, (n / 3) as i64],
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(renderer);
        let prepared = core::prepare(&d, Operation::VisibleCopy)?;
        check(
            d.snapshot() == before,
            "Visible copy preview mutated original",
        )?;
        check(
            prepared.state.layers[..d.layers.len()] == d.layers
                && prepared.state.scene.objects == d.scene.objects,
            "Visible copy consumed original pixels/objects",
        )?;
        let last = prepared.state.layers.last().unwrap();
        let actual = (0..6)
            .map(|n| {
                last.sample(3, 2, (n % 3) as i64, (n / 3) as i64).store(
                    d.color.depth,
                    d.color.dither,
                    [(n % 3) as i64, (n / 3) as i64],
                )
            })
            .collect::<Vec<_>>();
        check(
            actual == expected,
            "Visible copy lost precision/draft/ordered content",
        )?;
        d.commit_state(prepared.state)?;
        let after = d.snapshot();
        let file = root.join("visible-copy.drawer");
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        let reopened =
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
        check(
            reopened.snapshot() == after,
            "Visible copy precision and source order native roundtrip",
        )?;
        // The deferred copy must be last, after editable page objects.
        let renderer = crate::render::Renderer::new(&d);
        for (n, value) in expected.iter().copied().enumerate() {
            let a = match d.color.depth {
                Depth::Eight => Sample::from8(value.map(|v| v as u8)),
                Depth::Sixteen => Sample::from16(value),
            };
            let b = renderer.sample([(n % 3) as f64 + 0.5, (n / 3) as f64 + 0.5])?;
            let e = a.over(a, crate::layer_blend::Mode::Normal, 1.)?.store(
                d.color.depth,
                d.color.dither,
                [(n % 3) as i64, (n / 3) as i64],
            );
            let got = b.store(
                d.color.depth,
                d.color.dither,
                [(n % 3) as i64, (n / 3) as i64],
            );
            check(
                e.into_iter().zip(got).all(|(x, y)| x.abs_diff(y) <= 1),
                "Visible copy composed below objects",
            )?;
        }
        drop(renderer);
        check(
            d.undo() && d.snapshot() == before && d.redo() && d.snapshot() == after,
            "Visible copy Undo/Redo",
        )?;
    }
    // Alpha 1/65535 must not be mistaken for zero by the RGBA8 mirror.
    let mut d = fixture()?;
    let mut s = d.snapshot();
    s.color = Default::default();
    for l in &mut s.layers {
        l.precise = Some(
            l.pixels
                .iter()
                .map(|v| crate::color_precision::Sample::from8(*v).rgba16())
                .collect(),
        );
    }
    s.layers[1].precise.as_mut().unwrap()[0] = [1, 2, 3, 1];
    s.restore(&mut d);
    check(
        !core::prepare(&d, Operation::Empty)?
            .candidates
            .iter()
            .any(|v| v.0 == Key::Layer(Target::Raster(2))),
        "Low 16bit alpha deleted as empty",
    )?;
    Ok(())
}
fn boundaries() -> Result<(), String> {
    let mut d = fixture()?;
    // Hidden parent can be deleted only together with every child.
    let s = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Wrap {
            name: "Hidden group".into(),
            targets: vec![Target::Raster(2), Target::Raster(3)],
        },
    )?
    .state;
    s.restore(&mut d);
    d.scene.paper.folders[0].paper.hidden = true;
    let before = d.snapshot();
    let r = core::prepare(&d, Operation::Hidden)?;
    check(
        r.candidates
            .iter()
            .any(|v| v.0 == Key::Layer(Target::Folder(0))),
        "Hidden folder not included",
    )?;
    d.commit_state(r.state)?;
    check(
        !d.scene.paper.folder_active(0),
        "Deleted folder remains active",
    )?;
    check(d.undo() && d.snapshot() == before, "Folder cleanup Undo")?;
    d.scene.layer_properties.set(
        Target::Raster(2),
        p::Properties {
            position_locked: true,
            ..Default::default()
        },
    );
    let r = core::prepare(&d, Operation::Hidden)?;
    check(
        !r.candidates
            .iter()
            .any(|v| v.0 == Key::Layer(Target::Folder(0)) || v.0 == Key::Layer(Target::Raster(2))),
        "Locked descendant deleted through parent",
    )?;
    let mut track_doc = fixture()?;
    track_doc.scene.layer_properties.cells.clear();
    track_doc.scene.layer_properties.set(
        Target::Track(0),
        p::Properties {
            hidden: true,
            ..Default::default()
        },
    );
    let original = track_doc.snapshot();
    let prepared = core::prepare(&track_doc, Operation::Hidden)?;
    check(
        prepared.exposures == 3 && prepared.candidates.iter().any(|v| v.0 == Key::Track(1)),
        "Whole hidden timeline track impact",
    )?;
    track_doc.commit_state(prepared.state)?;
    check(
        track_doc.animation.as_ref().unwrap().tracks.is_empty()
            && track_doc.undo()
            && track_doc.snapshot() == original,
        "Whole track delete/Undo",
    )?;
    let mut effect_doc = fixture()?;
    effect_doc.scene.layer_properties.set(
        Target::Raster(2),
        p::Properties {
            blend: crate::layer_blend::Mode::Multiply,
            ..Default::default()
        },
    );
    let result = core::prepare(&effect_doc, Operation::Empty)?;
    check(
        !result
            .candidates
            .iter()
            .any(|v| v.0 == Key::Layer(Target::Raster(2))),
        "Non-Normal effect recipe deleted as empty",
    )?;
    let mut d = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    check(
        core::prepare(&d, Operation::Empty).is_err(),
        "Last raster removed",
    )?;
    d.scene.paper.layers.entry(0).or_default().hidden = true;
    check(
        core::prepare(&d, Operation::Hidden).is_err(),
        "Last hidden raster removed",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn layer_cleanup_copy_precision_locks_impact_native_single_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
