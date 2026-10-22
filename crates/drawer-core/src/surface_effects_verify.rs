use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    scene::Rect,
    surface_effects::{self as core, Entry, Light, Operation as O, Stack},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 25,
            height: 21,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for y in 6..15 {
        for x in 6..16 {
            if x < 12 || y > 10 {
                d.layers[0].pixels[y * 25 + x] = [
                    210,
                    (50 + x * 5) as u8,
                    90,
                    if x == 6 || y == 6 { 100 } else { 255 },
                ];
            }
        }
    }
    d.layers[0].pixels[2 * 25 + 2] = [255; 4];
    Ok(d)
}
fn stack(op: O) -> Stack {
    Stack {
        effects: vec![Entry {
            operation: op,
            enabled: true,
            scope: None,
        }],
        ..Default::default()
    }
}
fn run(d: &Document, s: &Stack) -> Result<core::Image, String> {
    core::render(
        d.layers[0].extent(d.width, d.height),
        s,
        DEFAULT_DOCUMENT_BUDGET,
        |p| Ok(d.layers[0].sample(d.width, d.height, p[0], p[1])),
        |_, _| Ok(()),
    )
}
fn shadow(blur: u32, angle: f64, distance: f64, opacity: f64) -> O {
    O::Shadow {
        blur,
        direction: angle,
        distance,
        color: [20, 70, 130],
        opacity,
    }
}
fn diff(a: &core::Image, b: &core::Image) -> bool {
    (0..21).any(|y| (0..25).any(|x| a.sample([x, y]).rgba16() != b.sample([x, y]).rgba16()))
}
fn alpha_sum(i: &core::Image) -> f64 {
    i.pixels.iter().map(|p| p[3] as f64).sum()
}
pub fn kernels() -> Result<(), String> {
    let d = fixture()?;
    let before = d.snapshot();
    let zero = run(&d, &stack(shadow(0, 0., 5., 0.)))?;
    for (i, p) in d.layers[0].pixels.iter().enumerate() {
        check(
            zero.sample([i as i64 % 25, i as i64 / 25]).rgba8() == *p,
            "Shadow opacity0 source invariant",
        )?;
    }
    let right = run(&d, &stack(shadow(0, 0., 4., 0.7)))?;
    let left = run(&d, &stack(shadow(0, 180., 4., 0.7)))?;
    check(
        right.sample([6, 2]).alpha() > 0. && left.sample([6, 2]).alpha() == 0.,
        "Asymmetric shadow direction / distance",
    )?;
    let blurred = run(&d, &stack(shadow(2, 0., 4., 0.7)))?;
    check(diff(&right, &blurred), "Shadow blur independent")?;
    let far = run(&d, &stack(shadow(0, 0., 80., 0.7)))?;
    check(
        far.sample([82, 2]).alpha() > 0.,
        "Off-canvas shadow retained",
    )?;
    check(
        diff(&right, &run(&d, &stack(shadow(0, 0., 0., 0.7)))?),
        "Distance0 remains independently adjustable",
    )?;
    for op in O::defaults() {
        let s = stack(op);
        let a = run(&d, &s)?;
        let b = run(&d, &s)?;
        check(
            a.pixels == b.pixels,
            "Surface kernels must be deterministic",
        )?;
    }
    let glow = |intensity, spread, blur| O::Glow {
        intensity,
        spread,
        blur,
        color: [255, 230, 150],
    };
    let g = run(&d, &stack(glow(1., 2, 0)))?;
    let soft = run(&d, &stack(glow(1., 2, 2)))?;
    let spread = run(&d, &stack(glow(1., 4, 0)))?;
    check(
        diff(&g, &soft) && diff(&g, &spread),
        "Glow spread and blur independent",
    )?;
    check(
        alpha_sum(&run(&d, &stack(glow(0.5, 2, 0)))?) < alpha_sum(&g),
        "Glow intensity contribution",
    )?;
    let emboss = |shadow, gloss, height, smoothing, light| O::Emboss {
        shadow,
        gloss,
        height,
        smoothing,
        light,
    };
    let a = run(&d, &stack(emboss(1., 0.7, 8., 0, Light::default())))?;
    for op in [
        emboss(0., 0.7, 8., 0, Light::default()),
        emboss(1., 0., 8., 0, Light::default()),
        emboss(1., 0.7, 2., 0, Light::default()),
        emboss(1., 0.7, 8., 2, Light::default()),
        emboss(1., 0.7, 8., 0, Light { x: 0.6, y: -0.8 }),
        emboss(1., 0.7, 8., 0, Light { x: -0.6, y: 0.8 }),
    ] {
        check(
            diff(&a, &run(&d, &stack(op))?),
            "Emboss controls / paired light must independently affect output",
        )?;
    }
    for y in 0..21 {
        for x in 0..25 {
            check(
                (a.sample([x, y]).alpha() - d.layers[0].sample(25, 21, x, y).alpha()).abs() < 1e-6,
                "Emboss alpha preserved",
            )?;
        }
    }
    let crystal = |offset, tiling, seed| O::Crystal {
        cell: 5,
        irregularity: 0.8,
        offset,
        tiling,
        seed,
    };
    let a = run(&d, &stack(crystal([0.; 2], true, u64::MAX - 1)))?;
    let b = run(&d, &stack(crystal([25., 21.], true, u64::MAX - 1)))?;
    check(
        !diff(&a, &b),
        "Tiled crystal offsets repeat at source period",
    )?;
    check(
        diff(&a, &run(&d, &stack(crystal([2., 0.], true, u64::MAX - 1)))?),
        "Crystal offset moves pattern",
    )?;
    check(
        diff(&a, &run(&d, &stack(crystal([0.; 2], true, 42)))?),
        "Crystal seed independent",
    )?;
    for cell in [1, 256] {
        run(
            &d,
            &stack(O::Crystal {
                cell,
                irregularity: 0.,
                offset: [0.; 2],
                tiling: false,
                seed: 0,
            }),
        )?;
    }
    let mut scoped = stack(shadow(0, 0., 4., 0.7));
    scoped.effects[0].scope = Some(core::Scope {
        rect: Some(Rect {
            min: [0., 0.],
            max: [5., 21.],
        }),
        ..Default::default()
    });
    let restricted = run(&d, &scoped)?;
    for y in 0..21 {
        for x in 5..25 {
            check(
                restricted.sample([x, y]).rgba8() == d.layers[0].sample(25, 21, x, y).rgba8(),
                "Frozen selection outside invariant",
            )?;
        }
    }
    let mut cancelled = 0;
    check(
        core::render(
            d.layers[0].extent(25, 21),
            &stack(shadow(8, 45., 8., 1.)),
            DEFAULT_DOCUMENT_BUDGET,
            |p| Ok(d.layers[0].sample(25, 21, p[0], p[1])),
            |_, _| {
                cancelled += 1;
                if cancelled > 4 {
                    Err("cancel".into())
                } else {
                    Ok(())
                }
            },
        )
        .is_err(),
        "CPU cancellation",
    )?;
    check(
        run(&d, &stack(shadow(33, 0., 1., 1.))).is_err(),
        "Invalid radius rejected",
    )?;
    check(d.snapshot() == before, "Kernels never mutate source")?;
    for raw in [
        r#"{"x":0.2}"#,
        r#"{"y":0.3}"#,
        r#"{"x":2,"y":0.3}"#,
        r#"{"x":"bad","y":0.3}"#,
    ] {
        let light: Light = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        check(
            light == Light::default(),
            "Invalid/incomplete light pair resets both axes",
        )?;
    }
    check(
        serde_json::from_str::<Light>(r#"{"x":0.2,"y":0.3}"#).map_err(|e| e.to_string())?
            == Light { x: 0.2, y: 0.3 },
        "Valid light pair restored",
    )?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    kernels()?;
    composition()?;
    let mut d = fixture()?;
    let original = d.snapshot();
    let mut s = stack(shadow(1, 0., 5., 0.7));
    s.effects.push(Entry {
        operation: O::defaults()[1],
        enabled: true,
        scope: None,
    });
    let state = core::prepare(&d, 0, s.clone(), |_, _| Ok(()))?;
    d.commit_state(state)?;
    check(
        d.layers == original.layers,
        "Live effects preserve source RGBA",
    )?;
    let r = crate::render::Renderer::new(&d);
    check(r.error().is_none(), "Live surface renderer")?;
    let sample = r.sample([22.5, 12.5])?.rgba16();
    drop(r);
    let file = root.join("surface-effects.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot()
            && crate::render::Renderer::new(&reopened)
                .sample([22.5, 12.5])?
                .rgba16()
                == sample,
        "Effects native roundtrip",
    )?;
    let mut old = std::fs::read(&file).map_err(|e| e.to_string())?;
    old[8..12].copy_from_slice(&73u32.to_le_bytes());
    let bad = root.join("surface-effects-old.drawer");
    std::fs::write(&bad, old).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&bad, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old native cannot silently reinterpret surface metadata",
    )?;
    check(
        d.undo() && d.snapshot() == original,
        "One Undo restores source and structure",
    )?;
    let baked = core::rasterize(&d, 0, s.clone(), |_, _| Ok(()))?;
    check(
        baked.layers.len() == 2
            && baked.layers[0] == original.layers[0]
            && baked.scene.surface_effects.baked.len() == 1,
        "Separate raster preserves source / recipe",
    )?;
    let baked_doc = crate::layer_merge::preview(&baked, 0)?;
    let live_doc =
        crate::layer_merge::preview(&core::prepare(&d, 0, s.clone(), |_, _| Ok(()))?, 0)?;
    let a = crate::render::Renderer::new(&baked_doc);
    let b = crate::render::Renderer::new(&live_doc);
    for y in 0..21 {
        for x in 0..25 {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            let av = a.sample(p)?.rgba8();
            let bv = b.sample(p)?.rgba8();
            check(
                av.iter().zip(bv).all(|(a, b)| a.abs_diff(b) <= 1),
                &format!("Raster result equals live preview at {x},{y}: {av:?} vs {bv:?}"),
            )?;
        }
    }
    let mut reverse = s.clone();
    reverse.effects.reverse();
    let a = run(&d, &s)?;
    let b = run(&d, &reverse)?;
    check(diff(&a, &b), "Effect order is observable")?;
    let mut empty = fixture()?;
    empty.layers[0].pixels.fill([0; 4]);
    for op in O::defaults() {
        check(
            alpha_sum(&run(&empty, &stack(op))?) == 0.,
            "Empty / fully transparent source",
        )?;
    }
    Ok(())
}
pub fn composition() -> Result<(), String> {
    use crate::{
        layer_effects::Mask,
        layer_properties::{self as p, Target},
        paper::{Folder, Node},
        selection::{Mask as Coverage, Run},
    };
    let mut d = fixture()?;
    let upper = d.layers[0].clone();
    let state = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "Clipped source".into(),
            parent: None,
        },
    )?
    .state;
    d.commit_state(state)?;
    d.layers[1] = upper;
    d.layers[0].pixels.fill([0; 4]);
    for y in 0..21 {
        for x in 0..12 {
            d.layers[0].pixels[y * 25 + x] = [60, 80, 120, 160];
        }
    }
    let base = p::raster(&d, 0);
    let top = p::raster(&d, 1);
    d.scene.clipping.bind(top, Some(base));
    let mask = |right, alpha| Mask {
        space: Default::default(),
        coverage: Coverage {
            runs: (0..21)
                .map(|y| Run {
                    x: 0,
                    y,
                    length: right,
                    alpha,
                })
                .collect(),
        },
        offset: [0.; 2],
        enabled: true,
        linked: false,
        inverted: false,
    };
    let Target::Raster(top_id) = top else {
        unreachable!()
    };
    d.scene
        .layer_effects
        .rasters
        .entry(top_id)
        .or_default()
        .mask = Some(mask(10, 128));
    let mut s = stack(shadow(0, 0., 8., 1.));
    d.scene.selection = Some(Rect {
        min: [0., 0.],
        max: [23., 21.],
    });
    s.effects[0].scope = Some(core::Scope::capture(&d));
    let before = d.snapshot();
    let state = core::prepare(&d, 1, s.clone(), |_, _| Ok(()))?;
    d.commit_state(state)?;
    let r = crate::float_render::Renderer::new(&d)?;
    let halo = r.surface_sample(1, [16, 12])?;
    check(
        halo.alpha() > 0.,
        "Source-mask/clipping shadow must extend outside source mask",
    )?;
    check(
        r.pixel([16.5, 12.5])?.alpha() > 0.,
        "Clipped effects must retain their halo",
    )?;
    drop(r);
    let base_effect = core::prepare(
        &d,
        0,
        stack(O::Glow {
            intensity: 2.,
            spread: 5,
            blur: 0,
            color: [255; 3],
        }),
        |_, _| Ok(()),
    )?;
    let expanded = crate::layer_merge::preview(&base_effect, 0)?;
    let r = crate::float_render::Renderer::new(&expanded)?;
    check(
        r.surface_sample(1, [16, 12])?.rgba16() == halo.rgba16(),
        "Clipping alpha must exclude base spatial effects / no cycles",
    )?;
    drop(r);
    d.scene.selection = Some(Rect {
        min: [0.; 2],
        max: [2.; 2],
    });
    check(
        crate::float_render::Renderer::new(&d)?
            .surface_sample(1, [16, 12])?
            .rgba16()
            == halo.rgba16(),
        "Later selection must not change captured scope",
    )?;
    let mut output = s.clone();
    output.output_mask = Some(mask(12, 255));
    let out =
        crate::layer_merge::preview(&core::prepare(&d, 1, output.clone(), |_, _| Ok(()))?, 0)?;
    check(
        crate::float_render::Renderer::new(&out)?
            .surface_sample(1, [16, 12])?
            .alpha()
            == 0.,
        "Output mask clips expanded halo",
    )?;
    output.output_mask.as_mut().unwrap().enabled = false;
    let out = crate::layer_merge::preview(&core::prepare(&d, 1, output, |_, _| Ok(()))?, 0)?;
    check(
        crate::float_render::Renderer::new(&out)?
            .surface_sample(1, [16, 12])?
            .alpha()
            > 0.,
        "Output-mask toggle restores halo",
    )?;
    let mut parent = d.snapshot();
    parent.scene.paper.folders.push(Folder {
        name: "Masked output".into(),
        children: vec![Node::Raster(0), Node::Raster(1)],
        paper: Default::default(),
        through: false,
    });
    parent.scene.paper.roots = vec![Node::Folder(0)];
    parent
        .scene
        .layer_effects
        .folders
        .entry(0)
        .or_default()
        .mask = Some(mask(12, 255));
    let parent = crate::layer_merge::preview(&parent, 0)?;
    check(
        crate::float_render::Renderer::new(&parent)?
            .pixel([16.5, 12.5])?
            .alpha()
            == 0.,
        "Parent mask clips complete effect output",
    )?;
    let raster = core::rasterize(&d, 1, s.clone(), |_, _| Ok(()))?;
    let mut erase = d.snapshot();
    erase.scene.paper.layers.entry(0).or_default().blend = crate::brush_composite::Mode::Erase;
    let erase = crate::layer_merge::preview(&erase, 0)?;
    check(
        core::rasterize(&erase, 1, s.clone(), |_, _| Ok(())).is_err(),
        "Backdrop-dependent clipping paper must not silently change appearance",
    )?;
    let baked = crate::layer_merge::preview(&raster, 0)?;
    let a = crate::render::Renderer::new(&baked);
    let b = crate::render::Renderer::new(&d);
    for y in 0..21 {
        for x in 0..25 {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            let a = a.sample(p)?.rgba8();
            let b = b.sample(p)?.rgba8();
            check(
                a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1),
                &format!("Clipping-run raster matches live at {x},{y}: {a:?}/{b:?}"),
            )?;
        }
    }
    let mut off = s.clone();
    off.effects[0].enabled = false;
    let off = crate::layer_merge::preview(&core::prepare(&d, 1, off, |_, _| Ok(()))?, 0)?;
    check(
        crate::float_render::Renderer::new(&off)?
            .pixel([16.5, 12.5])?
            .alpha()
            == 0.,
        "Effect toggle restores masked source",
    )?;
    let mut source = d.snapshot();
    source
        .scene
        .layer_effects
        .rasters
        .get_mut(&top_id)
        .unwrap()
        .mask
        .as_mut()
        .unwrap()
        .enabled = false;
    let unmasked = crate::layer_merge::preview(&source, 0)?;
    check(
        crate::float_render::Renderer::new(&unmasked)?
            .surface_sample(1, [18, 12])?
            .alpha()
            > crate::float_render::Renderer::new(&d)?
                .surface_sample(1, [18, 12])?
                .alpha(),
        "Source mask edits recompute effects",
    )?;
    // The existing border recipe also follows the source mask, before the ordered effects.
    let mut border = before.clone();
    border.scene.appearance.rasters.insert(
        top_id,
        crate::appearance::Style {
            enabled: true,
            border: 2,
            border_color: [255; 4],
            ..Default::default()
        },
    );
    let border = crate::layer_merge::preview(&border, 0)?;
    let border = crate::layer_merge::preview(
        &core::prepare(&border, 1, stack(shadow(0, 0., 8., 1.)), |_, _| Ok(()))?,
        0,
    )?;
    check(
        crate::float_render::Renderer::new(&border)?
            .surface_sample(1, [11, 12])?
            .alpha()
            > 0.,
        "Border is generated after source mask",
    )?;
    check(
        d.layers == before.layers,
        "Mask/selection/effect editing preserves all stored source pixels",
    )?;
    Ok(())
}
pub fn precision_and_limits(root: &std::path::Path) -> Result<(), String> {
    use crate::color_precision::{Depth, Sample};
    for kind in ["Shadow", "Glow", "Emboss", "Crystal"] {
        let op: O =
            serde_json::from_str(&format!("{{\"{kind}\":{{}}}}")).map_err(|e| e.to_string())?;
        check(
            op.valid() && op.identity(),
            "Missing parameters use safe identity defaults",
        )?;
    }
    let mut d = fixture()?;
    d.color = crate::color_storage::Config {
        depth: Depth::Sixteen,
        ..Default::default()
    };
    let mut samples = d.layers[0]
        .pixels
        .iter()
        .map(|p| Sample::from8(*p))
        .collect::<Vec<_>>();
    samples[0] = Sample::from16([50000, 21000, 30000, 1]);
    let e = d.layers[0].extent(d.width, d.height);
    crate::color_storage::set_samples(&mut d.layers[0], &d.color, e, samples)?;
    let original = d.snapshot();
    let state = core::prepare(&d, 0, stack(shadow(0, 0., 2., 0.5)), |_, _| Ok(()))?;
    d.commit_state(state)?;
    check(
        d.layers[0].precise == original.layers[0].precise,
        "Live effects preserve actual16-bit low alpha",
    )?;
    let rendered = crate::float_render::Renderer::new(&d)?;
    check(
        rendered.surface_sample(0, [2, 0])?.alpha() > 0.,
        "16-bit sub-preview-alpha casts shadow",
    )?;
    drop(rendered);
    let path = root.join("precise-surface.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Actual16-bit surface native reopen",
    )?;
    let baked = core::rasterize(&d, 0, stack(shadow(0, 0., 2., 0.5)), |_, _| Ok(()))?;
    check(
        baked.layers[1].precise.is_some() && baked.color == d.color,
        "Raster output retains document precision/profile",
    )?;
    let mut invalid = original.clone();
    invalid
        .scene
        .surface_effects
        .rasters
        .insert(999, stack(O::defaults()[0]));
    let mut invalid = crate::layer_merge::preview(&invalid, 0)?;
    check(
        crate::files::save(&mut invalid, &root.join("orphan-surface.drawer")).is_err(),
        "Orphan source reference rejected atomically",
    )?;
    let s = stack(shadow(32, 0., 256., 1.));
    check(
        core::extent(
            crate::document::PlacedRaster {
                width: 32768,
                height: 32768,
                offset: [0; 2],
            },
            &s,
        )
        .is_err(),
        "Oversized effect work rejected before allocation",
    )?;
    check(
        core::render(e, &s, 1, |_| Ok(Sample::default()), |_, _| Ok(())).is_err(),
        "Insufficient effect budget rejected",
    )?;
    let mut mono = fixture()?;
    mono.scene.catalog.rasters.entry(1).or_default().expression =
        crate::layer_search::Expression::Mono;
    core::prepare(&mono, 0, stack(O::defaults()[0]), |_, _| Ok(()))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_precision_missing_defaults_and_resource_limits() {
        let root = crate::test_support::TempDir::new();
        super::precision_and_limits(&root.0).unwrap();
    }
    #[test]
    fn source_output_parent_masks_and_clipping() {
        super::composition().unwrap();
    }
    #[test]
    fn kernels_controls_edges_and_seeds() {
        super::kernels().unwrap();
    }
    #[test]
    fn native_raster_and_undo() {
        let temp = crate::test_support::TempDir::new();
        super::verify(&temp.0).unwrap();
    }
}
