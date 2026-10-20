use crate::{
    color_precision::{Depth, Sample},
    color_storage::Config,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_effects::Mask,
    layer_merge::{self as merge, Operation, Settings},
    layer_properties::Target,
    selection::Run,
};
fn check(ok: bool, message: impl Into<String>) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn near(a: [u16; 4], b: [u16; 4]) -> bool {
    a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1)
}
fn units(p: Sample, depth: Depth) -> [u16; 4] {
    p.store(depth, Default::default(), [0; 2])
}
fn reference(a: Sample, b: Sample, ma: f64, mb: f64, depth: Depth) -> [u16; 4] {
    let aa = a.alpha() * ma;
    let ba = b.alpha() * mb;
    let alpha = ba + aa * (1. - ba);
    let max = depth.maximum() as f64;
    let quant = |v: f64| (v.clamp(0., 1.) * max + 0.5).floor() as u16;
    let alpha = quant(alpha);
    if alpha == 0 {
        return [0; 4];
    }
    let total = ba + aa * (1. - ba);
    let rgb = [0, 1, 2].map(|i| quant((b.rgb()[i] * ba + a.rgb()[i] * aa * (1. - ba)) / total));
    [rgb[0], rgb[1], rgb[2], alpha]
}
fn fixture(config: Config) -> Result<Document, String> {
    let mut d = Document::create_with_color(
        CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        config,
    )
    .map_err(|e| e.to_string())?;
    d.layers.push(d.layers[0].clone());
    d.layers[0].name = "Lower".into();
    d.layers[1].name = "Upper".into();
    d.scene.layer_ids.initialize(2)?;
    for (n, p) in [[12345, 23456, 34567, 12345], [54321, 43210, 32109, 45678]]
        .into_iter()
        .enumerate()
    {
        let sample = Sample::from16(p);
        if d.color.depth == Depth::Sixteen {
            d.layers[n].precise = Some(vec![p; 3]);
            d.layers[n].pixels = vec![sample.rgba8(); 3];
        } else {
            d.layers[n].precise = None;
            d.layers[n].pixels = vec![sample.rgba8(); 3];
        }
    }
    Ok(d)
}
fn mask(d: &Document, values: &[u8]) -> Mask {
    Mask {
        space: d.scene.transform.inverse(),
        coverage: crate::selection::Mask {
            runs: values
                .iter()
                .enumerate()
                .filter(|(_, a)| **a > 0)
                .map(|(x, a)| Run {
                    x: x as i32,
                    y: 0,
                    length: 1,
                    alpha: *a,
                })
                .collect(),
        },
        offset: [0.; 2],
        enabled: true,
        linked: true,
        inverted: false,
    }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for config in [
        Config::legacy(),
        Config {
            depth: Depth::Eight,
            ..Default::default()
        },
        Config::default(),
    ] {
        for lower in [Some(0), Some(128), Some(255), None] {
            for upper in [Some(0), Some(128), Some(255), None] {
                let mut d = fixture(config.clone())?;
                for (id, value) in [(1, lower), (2, upper)] {
                    if let Some(value) = value {
                        let m = mask(&d, &[value; 3]);
                        d.scene.layer_effects.rasters.entry(id).or_default().mask = Some(m);
                    }
                }
                let original = d.snapshot();
                let history = d.history_counts();
                let targets = vec![Target::Raster(1), Target::Raster(2)];
                let a = d.layers[0].sample_at(0);
                let b = d.layers[1].sample_at(0);
                let ma = lower.unwrap_or(255) as f64 / 255.;
                let mb = upper.unwrap_or(255) as f64 / 255.;
                let raw = reference(a, b, 1., 1., d.color.depth);
                let combined = merge::prepare(
                    &d,
                    Operation::Individual(targets.clone()),
                    Settings::default(),
                )?;
                check(
                    near(
                        units(combined.state.layers[0].sample_at(0), d.color.depth),
                        raw,
                    ),
                    "Individual raw source-over unexpectedly used a mask",
                )?;
                let m = combined
                    .state
                    .scene
                    .layer_effects
                    .get(combined.selected)
                    .and_then(|e| e.mask.as_ref())
                    .ok_or("Individual editable mask missing")?;
                check(
                    (m.factor([0.5, 0.5]) as f64 - ma.max(mb)).abs() < 1e-6,
                    "Individual mask must use max",
                )?;
                if lower.is_none() || upper.is_none() {
                    check(
                        m.factor([30.5, 30.5]) == 1.,
                        "Absent mask means one outside the old raster too",
                    )?;
                }
                let ordinary =
                    merge::prepare(&d, Operation::Selected(targets), Settings::default())?;
                check(
                    near(
                        units(ordinary.state.layers[0].sample_at(0), d.color.depth),
                        reference(a, b, ma, mb, d.color.depth),
                    ),
                    format!(
                        "Ordinary mask bake: {lower:?}/{upper:?}/{:?}",
                        d.color.depth
                    ),
                )?;
                check(
                    ordinary
                        .state
                        .scene
                        .layer_effects
                        .get(ordinary.selected)
                        .is_none(),
                    "Ordinary merge retained a baked mask",
                )?;
                if lower.is_some() {
                    let applied = crate::layer_masks::prepare(
                        &d,
                        Target::Raster(1),
                        crate::layer_masks::Command::Apply,
                    )?;
                    let expected = reference(Sample::default(), a, 0., ma, d.color.depth);
                    check(
                        near(
                            units(applied.state.layers[0].sample_at(0), d.color.depth),
                            expected,
                        ),
                        "Apply mask must multiply only its owner's alpha",
                    )?;
                    check(
                        applied.state.layers[1] == d.layers[1]
                            && applied
                                .state
                                .scene
                                .layer_effects
                                .get(Target::Raster(1))
                                .is_none(),
                        "Apply mask changed another source or kept mask",
                    )?;
                }
                check(
                    d.snapshot() == original && d.history_counts() == history,
                    "Preparing the three commands changed document/history",
                )?;
                d.commit_state(combined.state)?;
                let result = d.snapshot();
                let file = root.join("individual-mask-roundtrip.drawer");
                crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
                check(
                    crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                        .map_err(|e| e.to_string())?
                        .snapshot()
                        == result,
                    "Native pixel/mask separation lost",
                )?;
                check(
                    d.undo() && d.snapshot() == original && d.redo() && d.snapshot() == result,
                    "Individual combine one Undo/Redo",
                )?;
            }
        }
    }
    let mut d = fixture(Config::default())?;
    d.scene.transform = crate::scene::Affine([0., 1., -1., 0., 3., 0.]);
    let a = mask(&d, &[0, 128, 255]);
    let mut b = mask(&d, &[255, 128, 0]);
    b.linked = false;
    b.space = Default::default();
    b.offset = [0.25, 0.5];
    d.scene.layer_effects.rasters.entry(1).or_default().mask = Some(a);
    d.scene.layer_effects.rasters.entry(2).or_default().mask = Some(b);
    for inverted in [false, true] {
        for disabled in [false, true] {
            d.scene
                .layer_effects
                .rasters
                .get_mut(&2)
                .unwrap()
                .mask
                .as_mut()
                .unwrap()
                .inverted = inverted;
            d.scene
                .layer_effects
                .rasters
                .get_mut(&1)
                .unwrap()
                .mask
                .as_mut()
                .unwrap()
                .enabled = !disabled;
            let prepared = merge::prepare(
                &d,
                Operation::Individual(vec![Target::Raster(2), Target::Raster(1)]),
                Default::default(),
            )?;
            let m = prepared
                .state
                .scene
                .layer_effects
                .get(prepared.selected)
                .unwrap()
                .mask
                .as_ref()
                .unwrap();
            for y in -2..5 {
                for x in -2..7 {
                    let original = d
                        .scene
                        .transform
                        .inverse()
                        .map([x as f64 + 0.5, y as f64 + 0.5]);
                    let expected = d
                        .scene
                        .layer_effects
                        .mask(Target::Raster(1), original)
                        .max(d.scene.layer_effects.mask(Target::Raster(2), original));
                    check(
                        (m.factor(original) - expected).abs() < 1e-6,
                        "Rotated/inverted/disabled mask field differs from max",
                    )?;
                }
            }
        }
    }
    let mut d = fixture(Config::default())?;
    let m = mask(&d, &[128; 3]);
    d.scene.layer_effects.rasters.entry(1).or_default().mask = Some(m);
    let style = crate::appearance::Style {
        enabled: true,
        color: Some([10, 20, 30]),
        border: 1,
        rim: 0.5,
        ..Default::default()
    };
    d.scene.appearance.rasters.insert(1, style.clone());
    d.scene
        .layer_properties
        .rasters
        .entry(1)
        .or_default()
        .opacity = 37;
    let original = d.snapshot();
    let applied =
        crate::layer_masks::prepare(&d, Target::Raster(1), crate::layer_masks::Command::Apply)?;
    check(
        applied.state.scene.appearance == d.scene.appearance
            && applied.state.scene.layer_properties == d.scene.layer_properties
            && applied.state.scene.paper == d.scene.paper,
        "Apply mask baked unrelated display settings",
    )?;
    d.commit_state(applied.state)?;
    check(
        d.undo() && d.snapshot() == original,
        "Apply alpha mask one Undo",
    )?;
    let targets = vec![Target::Raster(1), Target::Raster(2)];
    check(
        merge::prepare(
            &d,
            Operation::Individual(targets.clone()),
            Default::default(),
        )
        .is_err(),
        "Non-100 opacity accepted",
    )?;
    d.scene
        .layer_properties
        .rasters
        .get_mut(&1)
        .unwrap()
        .opacity = 100;
    d.scene.layer_properties.rasters.get_mut(&1).unwrap().blend =
        crate::layer_blend::Mode::Multiply;
    check(
        merge::prepare(
            &d,
            Operation::Individual(targets.clone()),
            Default::default(),
        )
        .is_err(),
        "Non-Normal individual combine accepted",
    )?;
    d.scene.layer_properties.rasters.get_mut(&1).unwrap().blend = Default::default();
    d.scene
        .layer_effects
        .rasters
        .get_mut(&1)
        .unwrap()
        .adjustment = Some(crate::layer_effects::Adjustment {
        brightness: 0.1,
        ..Default::default()
    });
    check(
        merge::prepare(
            &d,
            Operation::Individual(targets.clone()),
            Default::default(),
        )
        .is_err(),
        "Correction individual combine accepted",
    )?;
    d.scene
        .layer_effects
        .rasters
        .get_mut(&1)
        .unwrap()
        .adjustment = None;
    check(
        merge::prepare(
            &d,
            Operation::Individual(vec![Target::Raster(1), Target::Raster(999)]),
            Default::default(),
        )
        .is_err(),
        "Missing lower raster accepted",
    )?;
    d.layers[0].locked = true;
    let original = d.snapshot();
    check(
        merge::prepare(&d, Operation::Individual(targets), Default::default()).is_err()
            && d.snapshot() == original,
        "Locked lower source changed",
    )?;
    check(
        merge::prepare(&d, Operation::Individual(vec![]), Default::default()).is_err(),
        "Missing lower source accepted",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn independent_pixels_and_masks_matrix_round_trip_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
