//! Owned local scatter, continuous ribbon, and procedural decoration masks.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Dabs,
    Scatter,
    Ribbon,
    Decoration,
}
impl Mode {
    pub const ALL: [Self; 4] = [Self::Dabs, Self::Scatter, Self::Ribbon, Self::Decoration];
    pub fn label(self) -> &'static str {
        match self {
            Self::Dabs => "Ordinary dabs",
            Self::Scatter => "Scatter",
            Self::Ribbon => "Connected ribbon",
            Self::Decoration => "Repeating decoration",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decoration {
    #[default]
    Star,
    Diamond,
    Leaf,
}
impl Decoration {
    pub const ALL: [Self; 3] = [Self::Star, Self::Diamond, Self::Leaf];
    pub fn label(self) -> &'static str {
        match self {
            Self::Star => "Owned star",
            Self::Diamond => "Owned diamond",
            Self::Leaf => "Owned leaf",
        }
    }
    pub fn distance(self, q: [f64; 2]) -> f64 {
        match self {
            Self::Star => q[0].hypot(q[1]) / (0.65 + 0.30 * (q[1].atan2(q[0]) * 5.0).cos()),
            Self::Diamond => q[0].abs() + q[1].abs(),
            Self::Leaf => q[0].abs() + q[1] * q[1],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub width: f64,
    pub count: u8,
    pub aspect: f64,
    pub seed: u32,
    pub interval: f64,
    pub decoration: Decoration,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Dabs,
            width: 16.0,
            count: 4,
            aspect: 1.0,
            seed: 0,
            interval: 1.5,
            decoration: Decoration::Star,
        }
    }
}
fn random(seed: u32, key: u64) -> f64 {
    let mut x = (seed as u64) ^ key.wrapping_add(1).wrapping_mul(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}
impl Settings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(self) -> bool {
        (0.0..=4096.0).contains(&self.width)
            && (1..=64).contains(&self.count)
            && (0.05..=1.0).contains(&self.aspect)
            && (0.25..=16.0).contains(&self.interval)
    }
    pub fn offset(self, group: u64, particle: u8, angle: f64) -> [f64; 2] {
        let key = group.wrapping_mul(128).wrapping_add(particle as u64 * 2);
        let radius = self.width * 0.5 * random(self.seed, key).sqrt();
        let a = std::f64::consts::TAU * random(self.seed, key + 1);
        let (s, c) = angle.to_radians().sin_cos();
        let x = radius * a.cos();
        let y = radius * a.sin() * self.aspect;
        [x * c - y * s, x * s + y * c]
    }
}
/// Normalized distance to a joined strip with round caps and interpolated width.
pub(crate) fn ribbon_distance(
    point: [f64; 2],
    from: ([f64; 2], f64),
    to: ([f64; 2], f64),
) -> (f64, f64) {
    let delta = [to.0[0] - from.0[0], to.0[1] - from.0[1]];
    let squared = delta[0] * delta[0] + delta[1] * delta[1];
    let t = if squared > 1e-12 {
        (((point[0] - from.0[0]) * delta[0] + (point[1] - from.0[1]) * delta[1]) / squared)
            .clamp(0.0, 1.0)
    } else {
        1.0
    };
    let radius = from.1 + (to.1 - from.1) * t;
    let center = [from.0[0] + delta[0] * t, from.0[1] + delta[1] * t];
    (
        (point[0] - center[0]).hypot(point[1] - center[1]) / radius.max(1e-9),
        radius,
    )
}

fn document() -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 64,
            height: 32,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())
}
fn style(pattern: Settings) -> crate::painting::Style {
    crate::painting::Style {
        pattern,
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 8.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [50, 100, 210, 180],
        opacity: 0.7,
        hardness: 1.0,
        spacing: 1.2,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    }
}
fn replay(
    d: &crate::document::Document,
    s: crate::painting::Style,
    points: &[[f64; 2]],
) -> Result<crate::mouse_pressure::Replay, String> {
    let mut r = crate::mouse_pressure::Replay::begin(d, 0, s, Default::default())?
        .configure_input(Default::default(), true)?;
    for point in points {
        r.push_input(
            crate::mouse_pressure::Sample {
                point: *point,
                micros: None,
            },
            1.0,
            0.0,
        )?;
    }
    Ok(r)
}
pub fn verify() -> Result<(), String> {
    for mode in [Mode::Scatter, Mode::Ribbon, Mode::Decoration] {
        for points in [
            &[[12.5, 12.5]][..],
            &[
                [8.5, 8.5],
                [34.5, 8.5],
                [8.5, 23.5],
                [34.5, 23.5],
                [8.5, 8.5],
            ][..],
        ] {
            let mut d = document()?;
            let before = d.snapshot();
            let s = style(Settings {
                mode,
                width: 16.0,
                count: 4,
                seed: 409,
                ..Default::default()
            });
            let r = replay(&d, s, points)?;
            drop(r.render(&d, false)?);
            if d.snapshot() != before {
                return Err("Pattern preview changed original".into());
            }
            r.render(&d, true)?.commit_mouse(&mut d, r.metadata())?;
            if d.layers[0].pixels.iter().all(|p| p[3] == 0) || d.history_counts() != (1, 0) {
                return Err(format!("{} dot/turn/crossing failed", mode.label()));
            }
            let after = d.snapshot();
            if !d.undo()
                || d.snapshot() != before
                || !d.redo()
                || d.snapshot() != after
                || !d.undo()
            {
                return Err("Pattern history diverged".into());
            }
            let r = replay(&d, s, points)?;
            r.render(&d, true)?.commit_mouse(&mut d, r.metadata())?;
            if d.snapshot() != after {
                return Err("Pattern seed or reconstruction changed".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dots_sharp_turn_self_crossing_seed_cancel_and_atomic_history() {
        verify().unwrap();
    }
    #[test]
    fn scatter_bounds_seed_count_and_zero_diameter() {
        let s = Settings {
            mode: Mode::Scatter,
            width: 40.0,
            count: 64,
            aspect: 0.25,
            seed: 518,
            ..Default::default()
        };
        for group in 0..10 {
            for n in 0..64 {
                let p = s.offset(group, n, 90.0);
                assert!(p[0].abs() <= 5.0 && p[1].abs() <= 20.0);
                assert_eq!(p, s.offset(group, n, 90.0));
            }
        }
        assert_ne!(
            s.offset(0, 0, 0.0),
            Settings { seed: 519, ..s }.offset(0, 0, 0.0)
        );
        assert_eq!(
            Settings { width: 0.0, ..s }.offset(40, 63, 110.0),
            [0.0, 0.0]
        );
        for bad in [f64::NAN, -1.0, 4096.1] {
            assert!(!Settings { width: bad, ..s }.valid());
        }
        assert!(!Settings { count: 0, ..s }.valid());
        assert!(!Settings { count: 65, ..s }.valid());
    }
    #[test]
    fn ribbon_connects_large_spacing_and_applies_global_coverage_once() {
        let mut results = Vec::new();
        for mode in [Mode::Dabs, Mode::Ribbon] {
            let mut d = document().unwrap();
            let mut s = style(Settings {
                mode,
                ..Default::default()
            });
            s.spacing = 10.0;
            let r = replay(&d, s, &[[8.5, 16.5], [54.5, 16.5]]).unwrap();
            r.render(&d, true)
                .unwrap()
                .commit_mouse(&mut d, r.metadata())
                .unwrap();
            results.push(d);
        }
        assert_eq!(results[0].composite_pixel(30, 16)[3], 0);
        for x in 8..=54 {
            assert_eq!(results[1].composite_pixel(x, 16), [50, 100, 210, 126]);
        }
        assert_eq!(
            ribbon_distance([5.0, 0.0], ([0.0; 2], 2.0), ([10.0, 0.0], 4.0)),
            (0.0, 3.0)
        );
    }
    #[test]
    fn decoration_masks_repeat_with_gaps_and_keep_alpha() {
        let mut masks = Vec::new();
        for decoration in Decoration::ALL {
            let mut d = document().unwrap();
            let s = style(Settings {
                mode: Mode::Decoration,
                decoration,
                interval: 2.0,
                ..Default::default()
            });
            let r = replay(&d, s, &[[8.5, 16.5], [56.5, 16.5]]).unwrap();
            r.render(&d, true)
                .unwrap()
                .commit_mouse(&mut d, r.metadata())
                .unwrap();
            assert!(d.composite_pixel(8, 16)[3] > 0);
            assert_eq!(d.composite_pixel(16, 16)[3], 0);
            assert!(d.layers[0].pixels.iter().all(|p| p[3] <= 126));
            let pixels = d.layers.remove(0).pixels;
            assert!(!masks.contains(&pixels));
            masks.push(pixels);
        }
    }
    #[test]
    fn preset_native_frozen_style_and_legacy_defaults() {
        use crate::{assets::BrushTip, brush_presets::Preset, workspaces::Snapshot};
        let root = crate::test_support::TempDir::new();
        let mut snap = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        snap.input.patterns[0] = Settings {
            mode: Mode::Scatter,
            count: 7,
            seed: 999,
            width: 31.0,
            aspect: 0.5,
            ..Default::default()
        };
        let p = Preset::capture(&snap, BrushTip::Pen, "Pattern brush").unwrap();
        let path = root.0.join("pattern.json");
        p.save(&path, false).unwrap();
        let p = Preset::read(&path).unwrap();
        assert_eq!(
            p.apply(&Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })
            .unwrap()
            .input
            .patterns[0],
            snap.input.patterns[0]
        );
        let mut d = document().unwrap();
        let r = replay(&d, style(p.pattern), &[[8.5, 16.5], [54.5, 16.5]]).unwrap();
        r.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, r.metadata())
            .unwrap();
        let before = d.snapshot();
        let path = root.0.join("pattern.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let next = crate::files::open(&path, 65536).unwrap();
        assert!(next.snapshot() == before);
        assert_eq!(next.scene.mouse_strokes[0].style.pattern, p.pattern);
        let mut old = serde_json::to_value(style(p.pattern)).unwrap();
        old.as_object_mut().unwrap().remove("pattern");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .pattern
                .is_default()
        );
        let mut old = serde_json::to_value(snap.input).unwrap();
        old.as_object_mut().unwrap().remove("patterns");
        assert_eq!(
            serde_json::from_value::<crate::tool_input::Settings>(old)
                .unwrap()
                .patterns,
            [Settings::default(); 7]
        );
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn outside_centerline_still_emits_visible_scatter_inside_canvas() {
        let mut d = document().unwrap();
        let mut s = style(Settings {
            mode: Mode::Scatter,
            width: 20.0,
            count: 64,
            seed: 33,
            ..Default::default()
        });
        s.shape.size = 4.0;
        s.spacing = 0.5;
        let r = replay(&d, s, &[[4.5, -6.5], [60.5, -6.5]]).unwrap();
        r.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, r.metadata())
            .unwrap();
        assert!((24..40).any(|x| (0..4).any(|y| d.composite_pixel(x, y)[3] > 0)));
    }
}
