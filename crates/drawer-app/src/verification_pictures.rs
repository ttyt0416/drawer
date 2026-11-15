//! Mathematically generated fixtures; no third-party painting assets.
use super::editor::check;
use drawer_core::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    input_replay::{Sample, Sequence},
    pointer_input::{Attributes, Event, Phase, Source},
};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize},
};
const BG: [u8; 4] = [40, 60, 80, 128];
fn sample(micros: u64, phase: Phase, position: [f64; 2]) -> Sample {
    Sample {
        micros,
        event: Event {
            source: Source::MOUSE,
            phase,
            position,
            pressure: None,
            attributes: Attributes::default(),
            origin: Default::default(),
        },
    }
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut cases = Vec::new();
    for name in [
        "straight",
        "circle",
        "corner",
        "very-slow",
        "fast",
        "pause",
        "lift-gap",
    ] {
        let points: Vec<[f64; 2]> = match name {
            "circle" => (0..=64)
                .map(|n| {
                    let a = n as f64 * std::f64::consts::TAU / 64.;
                    [32.5 + 16. * a.cos(), 24.5 + 16. * a.sin()]
                })
                .collect(),
            "corner" => vec![[8.5, 12.5], [32.5, 12.5], [32.5, 36.5], [55.5, 36.5]],
            "pause" => vec![
                [8.5, 24.5],
                [24.5, 24.5],
                [24.5, 24.5],
                [24.5, 24.5],
                [55.5, 24.5],
            ],
            "lift-gap" => vec![[8.5, 12.5], [55.5, 12.5], [8.5, 36.5], [55.5, 36.5]],
            _ => (0..48).map(|n| [8.5 + n as f64, 24.5]).collect(),
        };
        let mut s = Sequence {
            size: 1.,
            background: BG,
            color: [180, 20, 40, 255],
            pen_backend_available: false,
            ..Default::default()
        };
        for t in &mut s.pressure.tools {
            t.size = false;
            t.opacity = false;
        }
        for t in &mut s.mouse_pressure {
            t.enabled = false;
        }
        s.expected_strokes = if name == "lift-gap" { 2 } else { 1 };
        s.samples = points
            .iter()
            .enumerate()
            .map(|(n, p)| {
                let phase = if name == "lift-gap" {
                    if n % 2 == 0 { Phase::Down } else { Phase::Up }
                } else if n == 0 {
                    Phase::Down
                } else if n == points.len() - 1 {
                    Phase::Up
                } else {
                    Phase::Move
                };
                let time = match name {
                    "fast" => 0,
                    "very-slow" => n as u64 * 100_000,
                    "pause" => n as u64 * 1_000_000,
                    _ => n as u64 * 1000,
                };
                sample(time, phase, *p)
            })
            .collect();
        let input = root.join(format!("{name}.json"));
        s.save(&input).map_err(|e| e.to_string())?;
        let s = Sequence::read(&input).map_err(|e| e.to_string())?;
        let run = || crate::input_replay::run(&s, &AtomicBool::new(false), &AtomicUsize::new(0));
        let a = run()?;
        let b = run()?;
        check(
            a.record.passed && !a.record.incomplete_cancelled && a.pixels == b.pixels,
            "Picture replay/count/contact/exact repeat failed",
        )?;
        for p in [points[0], *points.last().unwrap()] {
            let [x, y] = p.map(|v| v.floor() as usize);
            check(
                a.pixels[y * 64 + x] != BG,
                "Picture expected start/end is missing",
            )?;
        }
        for x in 0..64 {
            check(
                a.pixels[x] == BG && a.pixels[47 * 64 + x] == BG,
                "Picture changed outside its expected vertical envelope",
            )?;
        }
        if name == "circle" {
            check(
                a.pixels[24 * 64 + 32] == BG,
                "Circle unexpectedly filled its center",
            )?;
        }
        if name == "lift-gap" {
            check(
                (0..64).all(|x| a.pixels[24 * 64 + x] == BG),
                "Pen lift connected separate strokes",
            )?;
        }
        let mut d = Document::create(
            CanvasSpec {
                width: 64,
                height: 48,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        d.layers[0].pixels.copy_from_slice(&a.pixels);
        let png = root.join(format!("{name}.png"));
        drawer_core::png_export::export(&d, &png, B).map_err(|e| e.to_string())?;
        let reopened = drawer_core::files::open(&png, B).map_err(|e| e.to_string())?;
        check(
            reopened.layers[0].pixels == a.pixels,
            "Generated PNG changed the fixture RGBA",
        )?;
        drawer_core::files::save(&mut d, &root.join(format!("{name}.drawer")))
            .map_err(|e| e.to_string())?;
        cases.push(serde_json::json!({"case":name,"input":s,"record":a.record,
            "pngSha256":format!("{:x}",Sha256::digest(std::fs::read(png).map_err(|e|e.to_string())?)),
            "expectedStart":points[0],"expectedEnd":points.last(),"outsideRowsUnchanged":[0,47],
            "liftGapUnchanged":name=="lift-gap","exactRepeat":true}));
    }
    super::strokes::edge_cases(root)?;
    let selection = super::masks::run(root)?;
    Ok(
        serde_json::json!({"source":"independently generated equations and coordinates in verification_pictures.rs",
        "thirdPartyAssets":false,"cases":cases,"selectionBoundaryChecks":selection,
        "zeroLengthLine":true,"oneBy32768Image":true,"tinyTransform":true,"merge256Layers":true}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn owned_pictures_endpoints_lift_gap_alpha_png_selection_and_degenerate_edges() {
        let root = crate::tests::TestDirectory::new();
        super::run(&root.0).unwrap();
    }
}
