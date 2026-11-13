use drawer_core::{
    assets::BrushTip,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    input_replay::{Sample, Sequence},
    pointer_input::{Event, Phase, Source},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize},
};
fn check(value: bool, message: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}
fn sample(micros: u64, phase: Phase, position: [f64; 2]) -> Sample {
    Sample {
        micros,
        event: Event {
            source: Source::MOUSE,
            phase,
            position,
            pressure: None,
            attributes: Default::default(),
            origin: Default::default(),
        },
    }
}
fn seed(tool: BrushTip, dot: bool) -> Vec<[u8; 4]> {
    (0..64 * 48)
        .map(|n| match tool {
            BrushTip::Eraser => [255; 4],
            BrushTip::Smudge if dot || n % 64 < 32 => [20, 40, 160, 255],
            BrushTip::Smudge => [220, 80, 20, 255],
            _ => [0; 4],
        })
        .collect()
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let mut cases = Vec::new();
    for tool in BrushTip::ALL {
        let mut reference = None;
        for name in ["dot", "short", "long", "fast-same-time", "rapid-dots"] {
            let dots = name == "dot" || name == "rapid-dots";
            let initial = seed(tool, dots);
            let mut sequence = Sequence {
                tool,
                size: 5.,
                pen_backend_available: false,
                ..Default::default()
            };
            for t in &mut sequence.pressure.tools {
                t.size = false;
                t.opacity = false;
            }
            for mouse in &mut sequence.mouse_pressure {
                mouse.enabled = false;
            }
            sequence.background = initial[0];
            sequence.samples = match name {
                "dot" => vec![
                    sample(0, Phase::Down, [20.5, 20.5]),
                    sample(1000, Phase::Up, [20.5, 20.5]),
                ],
                "short" => vec![
                    sample(0, Phase::Down, [28.5, 20.5]),
                    sample(1000, Phase::Up, [35.5, 20.5]),
                ],
                "rapid-dots" => (0..25)
                    .flat_map(|n| {
                        let p = [4.5 + (n % 5) as f64 * 12., 4.5 + (n / 5) as f64 * 8.];
                        [
                            sample(n * 2000, Phase::Down, p),
                            sample(n * 2000 + 1000, Phase::Up, p),
                        ]
                    })
                    .collect(),
                _ => (0..56)
                    .map(|n| {
                        sample(
                            if name == "fast-same-time" {
                                0
                            } else {
                                n * 1000
                            },
                            if n == 0 {
                                Phase::Down
                            } else if n == 55 {
                                Phase::Up
                            } else {
                                Phase::Move
                            },
                            [4.5 + n as f64, 20.5],
                        )
                    })
                    .collect(),
            };
            sequence.expected_strokes = if tool == BrushTip::Smudge && dots {
                0
            } else if name == "rapid-dots" {
                25
            } else {
                1
            };
            let output = crate::input_replay::run_seeded(
                &sequence,
                Default::default(),
                Some(&initial),
                &cancel,
                &progress,
            )
            .map_err(|e| format!("{tool:?}/{name}: {e}"))?;
            check(
                output.record.passed && !output.record.incomplete_cancelled,
                &format!(
                    "{tool:?}/{name}: stroke count or final contact mismatch: {}",
                    output.record.actual_strokes
                ),
            )?;
            let changed = output
                .pixels
                .iter()
                .zip(&initial)
                .filter(|(a, b)| a != b)
                .count();
            check(
                (changed > 0) == (sequence.expected_strokes > 0),
                &format!("{tool:?}/{name}: unexpected pixel change count {changed}"),
            )?;
            if !dots && tool != BrushTip::Smudge {
                let (start, end) = if name == "short" { (28, 35) } else { (4, 59) };
                for x in start..=end {
                    check(
                        (18..=23).any(|y| output.pixels[y * 64 + x] != initial[y * 64 + x]),
                        &format!("{tool:?}/{name}: footprint gap at x={x}"),
                    )?;
                }
            }
            if name == "long" {
                reference = Some(output.pixels.clone());
            }
            if name == "fast-same-time" {
                check(
                    reference.as_ref() == Some(&output.pixels),
                    &format!("{tool:?}: timestamp batching changed exact RGBA"),
                )?;
            }
            let again = crate::input_replay::run_seeded(
                &sequence,
                Default::default(),
                Some(&initial),
                &cancel,
                &progress,
            )?;
            check(
                again.pixels == output.pixels,
                &format!("{tool:?}/{name}: replay not deterministic"),
            )?;
            cases.push(serde_json::json!({"tool":format!("{tool:?}"),"case":name,
                "record":output.record,"changedPixels":changed,
                "alphaSum":output.pixels.iter().map(|p|u64::from(p[3])).sum::<u64>(),
                "seed":if tool==BrushTip::Smudge {serde_json::json!({"pattern":if dots{"solid"}else{"split-at-x32"},"left":[20,40,160,255],"right":[220,80,20,255]})}else{serde_json::json!({"solid":sequence.background})},
                "undoRedoSemanticComparison":true,"exactRepeat":true,
            }));
        }
    }
    crate::input_replay::smoke_ends()?;
    edge_cases(root)?;
    Ok(
        serde_json::json!({"mode":"owned CPU editor/input replay","scenarios":cases,
        "halfAlphaAndTaper":true,"zeroLengthLine":true,"oneBy32768Rotate":true,
        "tinyTransform":true,"maximum256LayerMerge":true}),
    )
}
fn blank(width: u32, height: u32) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width,
            height,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())
}
fn edge_cases(root: &Path) -> Result<(), String> {
    let mut line = blank(2, 2)?;
    let mut state = line.snapshot();
    state
        .scene
        .objects
        .push(drawer_core::scene::PageObject::Line {
            start: [0.5, 0.5],
            end: [0.5, 0.5],
            width_px: 1.,
            color: [80, 100, 120, 128],
        });
    line.commit_state(state)?;
    check(
        drawer_core::render::Renderer::new(&line).pixel([0.5, 0.5]) == [80, 100, 120, 128],
        "Zero-length line did not render its exact dot alpha",
    )?;
    let before = line.snapshot();
    let key = drawer_core::layer_search::Key::Object(line.scene.catalog.objects.id(0));
    let tiny = drawer_core::affine_transform::prepare(
        &line,
        key,
        false,
        drawer_core::vector_scope::Transform {
            translation: [1e-9, -1e-9],
            ..Default::default()
        },
        false,
        drawer_core::interpolation::Filter::Nearest,
    )?;
    line.commit_state(tiny.state)?;
    check(
        line.undo() && line.snapshot() == before,
        "Tiny line transform Undo changed source geometry",
    )?;
    let path = root.join("zero-line.drawer");
    drawer_core::files::save(&mut line, &path).map_err(|e| e.to_string())?;
    check(
        drawer_core::files::open(&path, B)
            .map_err(|e| e.to_string())?
            .snapshot()
            == before,
        "Zero-length line native reopen changed geometry",
    )?;
    let mut thin = blank(1, 32768)?;
    thin.layers[0].pixels[0] = [1, 2, 3, 128];
    thin.layers[0].pixels[32767] = [9, 8, 7, 255];
    let before = thin.snapshot();
    let rotated = drawer_core::sizing::prepare(
        &before,
        drawer_core::sizing::Change::Rotate90 { clockwise: true },
        B,
        &AtomicBool::new(false),
    )?;
    check(
        rotated.width == 32768
            && rotated.height == 1
            && rotated.layers[0].pixels[0] == [9, 8, 7, 255]
            && rotated.layers[0].pixels[32767] == [1, 2, 3, 128],
        "One-pixel long image lost endpoints on rotate",
    )?;
    thin.commit_state(rotated)?;
    check(
        thin.undo() && thin.snapshot() == before,
        "One-pixel long image Undo lost source",
    )?;
    let mut maximum = blank(1, 1)?;
    let mut state = maximum.snapshot();
    for n in 1..256 {
        let mut layer = state.layers[0].clone();
        layer.name = format!("Owned layer {n}");
        state.layers.push(layer);
    }
    state.layers[255].pixels[0] = [200, 100, 50, 255];
    state.scene.layer_ids.append(state.layers.len())?;
    maximum.commit_state(state)?;
    check(
        maximum.composite_pixel(0, 0) == [200, 100, 50, 255],
        "Maximum layer stack omitted the last layer",
    )?;
    let before = maximum.snapshot();
    let count = maximum.history_counts().0;
    let flat = drawer_core::layer_merge::prepare(
        &maximum,
        drawer_core::layer_merge::Operation::Flatten,
        Default::default(),
    )?;
    maximum.commit_state(flat.state)?;
    check(
        maximum.layers.len() == 1
            && maximum.composite_pixel(0, 0) == [200, 100, 50, 255]
            && maximum.history_counts().0 == count + 1
            && maximum.undo()
            && maximum.snapshot() == before,
        "256-layer merge/one Undo lost pixels, names, IDs or structure",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn all_basic_tools_mouse_contacts_pixels_batching_and_degenerate_edges() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
