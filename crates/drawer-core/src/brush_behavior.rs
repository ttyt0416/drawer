//! Optional brush sizing, transmittance correction and time emission rules.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub screen_size: bool,
    pub minimum_pixel: bool,
    pub spacing_density: bool,
    pub continuous: bool,
    pub rate: u32,
    pub fixed_scatter: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            screen_size: false,
            minimum_pixel: false,
            spacing_density: false,
            continuous: false,
            rate: 20,
            fixed_scatter: false,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (1..=200).contains(&self.rate)
    }
    pub fn document_size(self, size: f64, zoom: f64) -> Result<f64, String> {
        if !self.valid() || !size.is_finite() || size <= 0.0 || !zoom.is_finite() || zoom <= 0.0 {
            return Err("Invalid brush size/view scale".into());
        }
        Ok(self
            .minimum(if self.screen_size { size / zoom } else { size })
            .min(4096.0))
    }
    pub fn minimum(self, size: f64) -> f64 {
        if self.minimum_pixel {
            size.max(1.0)
        } else {
            size
        }
    }
    pub fn period(self) -> u32 {
        1_000_000 / self.rate.clamp(1, 200)
    }
    /// At the reference spacing .15 coverage is unchanged. Equal uniform coverage
    /// per distance has equal optical transmittance, independent of stamp count.
    pub fn corrected(self, amount: f64, ink_alpha: f64, spacing: f64) -> f64 {
        if !self.spacing_density || self.continuous || ink_alpha <= 0.0 {
            return amount;
        }
        let a = (amount * ink_alpha).clamp(0.0, 1.0);
        (1.0 - (1.0 - a).powf(spacing / 0.15)) / ink_alpha
    }
}

fn style() -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 1.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [220, 70, 30, 64],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    }
}
fn document() -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        32768,
    )
    .map_err(|e| e.to_string())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        mouse_pressure::{Replay, Sample},
        painting::Stroke,
    };
    let behavior = Settings {
        screen_size: true,
        ..Default::default()
    };
    for zoom in [0.5, 1.0, 2.0, 4.0] {
        let size = behavior.document_size(16.0, zoom)?;
        if (size * zoom - 16.0).abs() > 1e-9 {
            return Err("Screen diameter depends on zoom".into());
        }
    }
    if Settings::default().document_size(0.1, 2.0)? != 0.1
        || (Settings {
            minimum_pixel: true,
            ..Default::default()
        })
        .document_size(0.1, 2.0)?
            != 1.0
    {
        return Err("Minimum pixel option is not independent".into());
    }
    let d = document()?;
    let point = [8.5, 8.5];
    let index = 8 * 32 + 8;
    for (enabled, expected) in [(false, 0.1), (true, 1.0)] {
        let mut s = style();
        s.shape.size = 0.1;
        s.pattern.behavior.minimum_pixel = enabled;
        let mut stroke = Stroke::begin(&d, 0, s)?;
        stroke.direct_dab(&d, point, point, 1.0)?;
        if stroke.shape().size != expected {
            return Err("Minimum diameter was not applied to actual stroke shape".into());
        }
    }
    // Same optical distance: 20 half-spacing stamps versus 10 reference stamps.
    let mut alphas = vec![];
    for (spacing, count, corrected) in [
        (0.075, 20, true),
        (0.15, 10, true),
        (0.3, 5, true),
        (0.075, 20, false),
    ] {
        let mut doc = document()?;
        let mut s = style();
        s.pressure.opacity = false;
        s.spacing = spacing;
        s.pattern.behavior.spacing_density = corrected;
        let mut stroke = Stroke::begin(&doc, 0, s)?;
        for _ in 0..count {
            stroke.direct_dab(&doc, point, point, 1.0)?;
        }
        stroke.commit(&mut doc)?;
        alphas.push(doc.layers[0].pixels[index][3]);
    }
    if alphas[..3].iter().any(|a| a.abs_diff(alphas[1]) > 2) || alphas[3] <= alphas[0] + 8 {
        return Err(format!("Spacing optical correction differs: {alphas:?}"));
    }
    let mut s = style();
    s.pattern.behavior.continuous = true;
    s.pattern.behavior.rate = 20;
    let make = |clocks: &[u64], pressure: f64| -> Result<Replay, String> {
        let mut r = Replay::begin(&d, 0, s, Default::default())?
            .configure_input(Default::default(), true)?;
        r.push_input(
            Sample {
                point,
                micros: Some(0),
            },
            pressure,
            0.0,
        )?;
        for time in clocks {
            r.push_clock(Sample {
                point,
                micros: Some(*time),
            })?;
        }
        Ok(r)
    };
    for (clocks, expected) in [
        (&[][..], 64),
        (&[100000][..], 148),
        (&[100000, 200000][..], 195),
    ] {
        let r = make(clocks, 1.0)?;
        let stroke = r.render(&d, true)?;
        if stroke.pixel(&d, 0, &d.layers[0], 8, 8).unwrap_or([0; 4])[3] != expected {
            return Err("Held brush does not accumulate at fixed time intervals".into());
        }
    }
    let a = make(&[100000], 0.5)?;
    let b = make(&[10000, 50000, 70000, 100000], 0.5)?;
    // Replay is tied to its source document, so commit both into clones of that source.
    let mut sa = a.render(&d, true)?;
    let sb = b.render(&d, true)?;
    for y in 0..d.height {
        for x in 0..d.width {
            if sa.pixel(&d, 0, &d.layers[0], x, y) != sb.pixel(&d, 0, &d.layers[0], x, y) {
                return Err("Time segmentation changed native pressure result".into());
            }
        }
    }
    let mut source = d;
    sa.finish_sample(&source)?;
    sa.commit_mouse(&mut source, a.metadata())?;
    if source.layers[0].pixels[index][3] != 84 || source.history_counts() != (1, 0) {
        return Err("Native pressure or one-session history lost".into());
    }
    let pixels = source.layers[0].pixels.clone();
    if !source.undo() || source.layers[0].pixels.iter().any(|p| p[3] != 0) {
        return Err("Timed undo did not restore the original".into());
    }
    if !source.redo() || source.layers[0].pixels != pixels {
        return Err("Timed brush redo recalculated pixels".into());
    }
    crate::files::save(&mut source, &root.join("continuous-brush.drawer"))
        .map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&root.join("continuous-brush.drawer"), 32768)
        .map_err(|e| e.to_string())?;
    if reopened.layers[0].pixels != pixels
        || reopened.scene.mouse_strokes[0].style.pattern.behavior != s.pattern.behavior
    {
        return Err("Time behavior/native pixels not preserved".into());
    }
    let pattern = crate::brush_pattern::Settings {
        behavior: Settings {
            fixed_scatter: true,
            ..Default::default()
        },
        ..Default::default()
    };
    for tip in [0, 1, 1024] {
        for particle in 0..pattern.count {
            if pattern.offset_for_tip(0, particle, tip, 0.0)
                != pattern.offset_for_tip(999, particle, tip, 0.0)
            {
                return Err("Fixed scatter offsets changed per emission".into());
            }
        }
    }
    if pattern.offset_for_tip(0, 0, 1, 0.0) == pattern.offset_for_tip(0, 0, 2, 0.0) {
        return Err("Fixed scatter did not distinguish tips".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizing_density_and_time_boundaries() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn huge_clock_and_cancellation_leave_original_untouched() {
        let d = document().unwrap();
        let before = d.snapshot();
        let mut s = style();
        s.pattern.behavior.continuous = true;
        s.pattern.behavior.rate = 200;
        let mut r = crate::mouse_pressure::Replay::begin(&d, 0, s, Default::default())
            .unwrap()
            .configure_input(Default::default(), true)
            .unwrap();
        r.push_input(
            crate::mouse_pressure::Sample {
                point: [8.5, 8.5],
                micros: Some(0),
            },
            1.0,
            0.0,
        )
        .unwrap();
        r.push_clock(crate::mouse_pressure::Sample {
            point: [8.5, 8.5],
            micros: Some(u64::MAX),
        })
        .unwrap();
        assert!(r.render(&d, true).is_err());
        assert!(r.render_cancel(&d, true, || true).is_err());
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
    }
    #[test]
    fn legacy_bounds_and_optical_reference() {
        let mut value = serde_json::to_value(crate::brush_pattern::Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("behavior");
        assert_eq!(
            serde_json::from_value::<crate::brush_pattern::Settings>(value)
                .unwrap()
                .behavior,
            Settings::default()
        );
        for rate in [0, 201, u32::MAX] {
            assert!(
                !(Settings {
                    rate,
                    ..Default::default()
                })
                .valid()
            );
        }
        let s = Settings {
            spacing_density: true,
            ..Default::default()
        };
        for a in [0.0, 0.1, 0.5, 1.0] {
            assert!((s.corrected(a, 1.0, 0.15) - a).abs() < 1e-12);
            assert!((s.corrected(a, 1.0, 0.3) - (1.0 - (1.0 - a).powi(2))).abs() < 1e-12);
        }
        assert!(s.document_size(1.0, f64::NAN).is_err());
        assert!(s.document_size(0.0, 1.0).is_err());
    }
}
