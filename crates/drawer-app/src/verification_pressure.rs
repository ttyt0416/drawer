use super::editor::{Editor, check};
use drawer_core::{
    input_replay::{Sample, Sequence},
    pointer_input::{Event, Kind, Phase, Source},
    pressure::{Mapping, Response},
};
use std::sync::atomic::{AtomicBool, AtomicUsize};
const SOURCE: Source = Source {
    kind: Kind::Pen,
    device: 295,
    contact: 1,
};
fn event(phase: Phase, position: [f64; 2], pressure: f64) -> Event {
    Event {
        source: SOURCE,
        phase,
        position,
        pressure: Some(pressure),
        attributes: Default::default(),
        origin: Default::default(),
    }
}
fn mapping() -> Mapping {
    Mapping {
        size: false,
        opacity: true,
        response: Response {
            minimum: 0.25,
            maximum: 0.75,
            curve: [0., 0.1, 0.25, 0.8, 1.],
        },
        ..Default::default()
    }
}
fn setup() -> Result<Editor, String> {
    let mut e = Editor::new(1.5)?;
    e.app.panels.settings.brush_size = 8.;
    e.app.panels.settings.brush_opacity = [1.; 7];
    e.app.panels.settings.brush_hardness = [1.; 7];
    e.app.workspaces.store.current.pressure.tools[0] = mapping();
    e.app.workspaces.store.current.input.profiles[0].minimum_opacity = 0.;
    Ok(e)
}
fn send(e: &mut Editor, phase: Phase, point: [f64; 2], pressure: f64) -> Result<(), String> {
    let p = e.point(point)?;
    e.app.drawing.virtual_events(
        vec![event(phase, [f64::from(p.x), f64::from(p.y)], pressure)],
        true,
    );
    e.frame(vec![]);
    Ok(())
}
pub(super) fn run() -> Result<serde_json::Value, String> {
    let mut steady = vec![];
    for (raw, alpha) in [(0., 0u8), (0.25, 0), (0.5, 64), (0.75, 255), (1., 255)] {
        let mut e = setup()?;
        let before = e.app.documents[0].snapshot();
        send(&mut e, Phase::Down, [20.5, 20.5], raw)?;
        send(&mut e, Phase::Up, [20.5, 20.5], raw)?;
        let actual = e.app.documents[0].layers[0].pixels[20 * 64 + 20][3];
        check(
            actual == alpha,
            "Virtual pressure curve did not reach its independently expected pixel alpha",
        )?;
        let d = &mut e.app.documents[0];
        if alpha == 0 {
            check(
                d.snapshot() == before && d.history_counts() == (0, 0),
                "Zero pressure changed pixels or history",
            )?;
        } else {
            let after = d.snapshot();
            check(
                d.history_counts() == (1, 0)
                    && d.undo()
                    && d.snapshot() == before
                    && d.redo()
                    && d.snapshot() == after,
                "Virtual pressure Undo/Redo differs",
            )?;
        }
        check(
            !e.app.drawing.pending(),
            "Steady pressure left an active contact",
        )?;
        steady.push(serde_json::json!({"raw":raw,"expectedAlpha":alpha,"actualAlpha":actual}));
    }
    let invalid = [
        ("negative", -1.),
        ("above-one", 2.),
        ("finite-negative-maximum", -f64::MAX),
        ("finite-maximum", f64::MAX),
        ("NaN", f64::NAN),
        ("positive-infinity", f64::INFINITY),
        ("negative-infinity", f64::NEG_INFINITY),
    ];
    let mut rejected = vec![];
    for (label, value) in invalid {
        for phase in [Phase::Down, Phase::Move, Phase::Up] {
            let mut e = setup()?;
            let before = e.app.documents[0].snapshot();
            if phase != Phase::Down {
                send(&mut e, Phase::Down, [10.5, 20.5], 1.)?;
                send(&mut e, Phase::Move, [30.5, 20.5], 0.5)?;
                check(
                    e.app.drawing.pending(),
                    "Bad-pressure fixture never started",
                )?;
            }
            send(&mut e, phase, [40.5, 20.5], value)?;
            check(
                !e.app.drawing.pending()
                    && e.app.documents[0].snapshot() == before
                    && e.app.documents[0].history_counts() == (0, 0),
                "Invalid pen pressure failed to reject/cancel the whole pending contact",
            )?;
            send(&mut e, Phase::Move, [45.5, 20.5], 1.)?;
            send(&mut e, Phase::Up, [45.5, 20.5], 1.)?;
            check(
                e.app.documents[0].snapshot() == before,
                "Valid move/up resumed a rejected contact without a new Down",
            )?;
            send(&mut e, Phase::Down, [10.5, 30.5], 1.)?;
            send(&mut e, Phase::Up, [10.5, 30.5], 1.)?;
            let d = &mut e.app.documents[0];
            check(
                d.scene.valid()
                    && d.history_counts() == (1, 0)
                    && d.undo()
                    && d.snapshot() == before,
                "Fresh valid pressure did not restore normal state after invalid data",
            )?;
            let curve = mapping().response.map(value);
            let disposition = if value.is_finite() {
                let want = if value < 0. { 0. } else { 1. };
                check(
                    curve.as_ref().is_ok_and(|p| *p == want),
                    "Finite out-of-range curve input did not clamp safely",
                )?;
                "curve clamped; raw event rejected"
            } else {
                check(curve.is_err(), "Non-finite curve input was accepted")?;
                "curve rejected; raw event rejected"
            };
            rejected.push(serde_json::json!({"input":label,"ieee754Bits":format!("{:016x}",value.to_bits()),"phase":format!("{phase:?}"),"disposition":disposition,"pendingRollbackExact":true,"freshPressRecovered":true}));
        }
    }
    let values = [0., 1., 0., 1., 0.001, 0.999, 0.5, 1.];
    let mut sequence = Sequence {
        expected_strokes: 1,
        size: 8.,
        pen_backend_available: true,
        ..Default::default()
    };
    sequence.pressure.tools[0] = mapping();
    sequence.profiles[0].minimum_opacity = 0.;
    sequence.samples = values
        .into_iter()
        .enumerate()
        .map(|(n, value)| Sample {
            micros: n as u64 * 1000,
            event: event(
                if n == 0 {
                    Phase::Down
                } else if n == values.len() - 1 {
                    Phase::Up
                } else {
                    Phase::Move
                },
                [8.5 + n as f64 * 6., 20.5],
                value,
            ),
        })
        .collect();
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    let a = crate::input_replay::run(&sequence, &cancel, &progress)?;
    let b = crate::input_replay::run(&sequence, &cancel, &progress)?;
    check(
        a.pixels == b.pixels && a.record.actual_strokes == 1,
        "Abrupt virtual pressure was not deterministic or one undoable stroke",
    )?;
    Ok(
        serde_json::json!({"steadyCurveCases":steady,"invalidRawCases":rejected,"abruptPressureRecord":a.record,"exactAbruptRepeat":true,"rawNonFiniteEncoding":"named IEEE754 bits; no JSON NaN/Infinity values","physicalPenValidated":false}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn virtual_extrema_curve_pixels_nonfinite_contact_rollback_fresh_press_and_abrupt_replay() {
        super::run().unwrap();
    }
}
