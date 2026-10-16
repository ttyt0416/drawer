//! Independent, bounded brush-size factors and document-space orientation.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    #[default]
    Fixed,
    Stroke,
    Pen,
    ScatterOverall,
    ScatterCenter,
}
impl Direction {
    pub const ALL: [Self; 5] = [
        Self::Fixed,
        Self::Stroke,
        Self::Pen,
        Self::ScatterOverall,
        Self::ScatterCenter,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Stroke => "Stroke direction",
            Self::Pen => "Pen direction",
            Self::ScatterOverall => "Scatter overall direction",
            Self::ScatterCenter => "Toward scatter center",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    pub enabled: bool,
    pub minimum: f64,
    pub maximum: f64,
    pub curve: [f64; 5],
    pub output_minimum: f64,
    pub output_maximum: f64,
    pub missing: f64,
}
impl Default for Curve {
    fn default() -> Self {
        Self {
            enabled: false,
            minimum: 0.0,
            maximum: 1.0,
            curve: [0.0, 0.25, 0.5, 0.75, 1.0],
            output_minimum: 0.0,
            output_maximum: 1.0,
            missing: 1.0,
        }
    }
}
impl Curve {
    pub fn valid(self, limit: f64) -> bool {
        (0.0..=limit).contains(&self.minimum)
            && (0.0..=limit).contains(&self.maximum)
            && self.minimum < self.maximum
            && (0.0..=1.0).contains(&self.output_minimum)
            && (self.output_minimum..=1.0).contains(&self.output_maximum)
            && (0.0..=1.0).contains(&self.missing)
            && self.curve.iter().all(|v| (0.0..=1.0).contains(v))
    }
    pub fn map(self, value: Option<f64>) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        let Some(value) = value.filter(|v| v.is_finite()) else {
            return self.missing;
        };
        let x = ((value - self.minimum) / (self.maximum - self.minimum)).clamp(0.0, 1.0) * 4.0;
        let n = (x.floor() as usize).min(3);
        self.output_minimum
            + (self.output_maximum - self.output_minimum)
                * (self.curve[n] + (self.curve[n + 1] - self.curve[n]) * (x - n as f64))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub direction: Direction,
    pub fallback_angle: f64,
    pub scatter_angle: f64,
    pub spacing_with_size: bool,
    pub pressure: Curve,
    pub tilt: Curve,
    pub speed: Curve,
    pub random_minimum: f64,
    pub seed: u32,
    pub minimum_size: f64,
    pub maximum_size: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            direction: Default::default(),
            fallback_angle: 0.0,
            scatter_angle: 0.0,
            spacing_with_size: true,
            pressure: Curve::default(),
            tilt: Curve {
                maximum: 90.0,
                ..Default::default()
            },
            speed: Curve {
                maximum: 2000.0,
                ..Default::default()
            },
            random_minimum: 1.0,
            seed: 0,
            minimum_size: 0.0,
            maximum_size: 4096.0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub tilt: Option<f64>,
    pub bearing: Option<f64>,
    pub speed: Option<f64>,
}
impl Input {
    pub fn from_tilt(tilt: Option<[f64; 2]>, bearing: f64) -> Self {
        let Some(tilt) = tilt.filter(|v| {
            v.iter()
                .all(|x| x.is_finite() && x.abs() <= std::f64::consts::FRAC_PI_2)
        }) else {
            return Self::default();
        };
        let magnitude = tilt[0].tan().hypot(tilt[1].tan()).atan().to_degrees();
        Self {
            tilt: Some(magnitude),
            bearing: if magnitude > 1e-9 {
                Some(bearing)
            } else {
                None
            },
            speed: None,
        }
    }

    pub fn valid(self) -> bool {
        self.tilt.is_none_or(|v| (0.0..=90.0).contains(&v))
            && self
                .bearing
                .is_none_or(|v| v.is_finite() && v.abs() <= 360.0)
            && self
                .speed
                .is_none_or(|v| v.is_finite() && (0.0..=1e18).contains(&v))
    }
    pub fn interpolate(self, to: Self, t: f64) -> Self {
        let linear = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => Some(a + (b - a) * t),
            _ => {
                if t >= 1.0 {
                    b
                } else {
                    a
                }
            }
        };
        let bearing = match (self.bearing, to.bearing) {
            (Some(a), Some(b)) => Some(
                (a + ((b - a + 180.0).rem_euclid(360.0) - 180.0) * t + 180.0).rem_euclid(360.0)
                    - 180.0,
            ),
            _ => {
                if t >= 1.0 {
                    to.bearing
                } else {
                    self.bearing
                }
            }
        };
        Self {
            tilt: linear(self.tilt, to.tilt),
            bearing,
            speed: linear(self.speed, to.speed),
        }
    }
}
fn random(seed: u32, index: u64) -> f64 {
    let mut x = (seed as u64).wrapping_add(index.wrapping_mul(0x9e3779b97f4a7c15));
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}
impl Settings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(self) -> bool {
        self.pressure.valid(1.0)
            && self.tilt.valid(90.0)
            && self.speed.valid(1e6)
            && [self.fallback_angle, self.scatter_angle]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 180.0)
            && (0.0..=1.0).contains(&self.random_minimum)
            && (0.0..=4096.0).contains(&self.minimum_size)
            && (0.1..=4096.0).contains(&self.maximum_size)
            && self.minimum_size <= self.maximum_size
    }
    pub fn size(self, base: f64, pressure: f64, input: Input, index: u64) -> f64 {
        let factor = self.pressure.map(Some(pressure))
            * self.tilt.map(input.tilt)
            * self.speed.map(input.speed);
        let jitter = self.random_minimum + (1.0 - self.random_minimum) * random(self.seed, index);
        (base * factor * jitter).clamp(self.minimum_size, self.maximum_size)
    }
    pub fn angle(
        self,
        heading: Option<f64>,
        input: Input,
        particle: Option<[f64; 2]>,
        legacy: f64,
    ) -> f64 {
        match self.direction {
            Direction::Fixed => self.fallback_angle + legacy,
            Direction::Stroke => heading.unwrap_or(self.fallback_angle),
            Direction::Pen => input.bearing.unwrap_or(self.fallback_angle),
            Direction::ScatterOverall => self.scatter_angle,
            Direction::ScatterCenter => particle
                .filter(|p| p[0].hypot(p[1]) > 1e-9)
                .map_or(self.fallback_angle, |p| p[1].atan2(p[0]).to_degrees()),
        }
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
            size: 8.0,
            aspect: 0.25,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [50, 100, 210, 255],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.5,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
fn document() -> Result<crate::document::Document, String> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 48,
            height: 24,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())
}
/// Independent geometry checks shared with the installed application.
pub fn verify() -> Result<(), String> {
    use crate::painting::Stroke;
    let d = document()?;
    let before = d.snapshot();
    let mut s = style();
    s.dynamics.direction = Direction::Stroke;
    s.dynamics.fallback_angle = 32.0;
    let mut st = Stroke::begin(&d, 0, s)?;
    st.sample(&d, [12.5, 12.5])?;
    if st.shape().angle != 32.0 {
        return Err("Zero-length direction fallback failed".into());
    }
    st.sample(&d, [22.5, 12.5])?;
    if st.shape().angle != 0.0 {
        return Err("Forward stroke direction failed".into());
    }
    st.sample(&d, [12.5, 12.5])?;
    if st.shape().angle.abs() != 180.0 {
        return Err("Reversed stroke direction failed".into());
    }
    s.dynamics.direction = Direction::Pen;
    let mut st = Stroke::begin(&d, 0, s)?;
    st.sample(&d, [12.5, 12.5])?;
    if st.shape().angle != 32.0 {
        return Err("Missing tilt direction fallback failed".into());
    }
    let input = Input::from_tilt(Some([0.0, std::f64::consts::FRAC_PI_4]), 90.0);
    st.set_dynamic_input(input)?;
    st.sample(&d, [12.5, 12.5])?;
    if st.shape().angle != 90.0 {
        return Err("Pen direction failed".into());
    }
    for (kind, expected) in [(0, 2.0), (1, 4.0), (2, 4.0), (3, 0.5)] {
        let mut s = style();
        s.dynamics.pressure.enabled = kind == 0 || kind == 3;
        s.dynamics.tilt.enabled = kind == 1 || kind == 3;
        s.dynamics.speed.enabled = kind == 2 || kind == 3;
        let mut st = Stroke::begin(&d, 0, s)?;
        st.set_dynamic_input(Input {
            tilt: Some(45.0),
            speed: Some(1000.0),
            bearing: None,
        })?;
        st.sample_pressure(&d, [12.5, 12.5], 0.25)?;
        if (st.shape().size - expected).abs() > 1e-9 {
            return Err(format!(
                "Independent size factor {kind} failed: {}",
                st.shape().size
            ));
        }
    }
    s = style();
    s.dynamics.direction = Direction::ScatterCenter;
    let mut st = Stroke::begin(&d, 0, s)?;
    st.direct_dab(&d, [20.5, 12.5], [12.5, 12.5], 1.0)?;
    if st.shape().angle.abs() != 180.0 {
        return Err("Scatter particle did not face center".into());
    }
    s.dynamics.direction = Direction::ScatterOverall;
    s.dynamics.scatter_angle = 70.0;
    let mut st = Stroke::begin(&d, 0, s)?;
    st.direct_dab(&d, [20.5, 12.5], [12.5, 12.5], 1.0)?;
    if st.shape().angle != 70.0 {
        return Err("Scatter overall angle was coupled to particle".into());
    }
    if d.snapshot() != before {
        return Err("Dynamics preview changed the original".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_stroke_directions_and_independent_input_factors() {
        verify().unwrap();
    }
    #[test]
    fn curves_ranges_random_multiply_then_clamp_and_missing_inputs() {
        let mut s = Settings::default();
        s.pressure.enabled = true;
        s.tilt.enabled = true;
        s.speed.enabled = true;
        let i = Input {
            tilt: Some(45.0),
            speed: Some(1000.0),
            bearing: None,
        };
        assert_eq!(s.size(8.0, 0.25, i, 0), 0.5);
        s.minimum_size = 2.0;
        assert_eq!(s.size(8.0, 0.25, i, 0), 2.0);
        s.maximum_size = 3.0;
        assert_eq!(s.size(40.0, 1.0, i, 0), 3.0);
        s.tilt.missing = 0.2;
        s.speed.missing = 0.5;
        s.minimum_size = 0.0;
        s.maximum_size = 4096.0;
        assert!((s.size(10.0, 0.5, Input::default(), 0) - 0.5).abs() < 1e-10);
        s = Settings::default();
        s.random_minimum = 0.2;
        s.seed = 23;
        let a = (0..20)
            .map(|n| s.size(8.0, 1.0, Input::default(), n))
            .collect::<Vec<_>>();
        assert!(a.iter().all(|n| (1.6..=8.0).contains(n)));
        assert!(a.windows(2).any(|p| p[0] != p[1]));
        assert_eq!(
            a,
            (0..20)
                .map(|n| s.size(8.0, 1.0, Input::default(), n))
                .collect::<Vec<_>>()
        );
        s.seed += 1;
        assert_ne!(
            a,
            (0..20)
                .map(|n| s.size(8.0, 1.0, Input::default(), n))
                .collect::<Vec<_>>()
        );
        let c = Curve {
            enabled: true,
            curve: [1.0, 0.75, 0.5, 0.25, 0.0],
            ..Default::default()
        };
        assert!(c.valid(1.0));
        assert_eq!(c.map(Some(0.25)), 0.75);
        assert!(
            !Curve {
                maximum: f64::NAN,
                ..c
            }
            .valid(1.0)
        );
        assert!(
            !Curve {
                minimum: 1.0,
                maximum: 1.0,
                ..c
            }
            .valid(1.0)
        );
        assert!(
            !Settings {
                minimum_size: 5.0,
                maximum_size: 4.0,
                ..s
            }
            .valid()
        );
        let a = Input {
            bearing: Some(179.0),
            ..Default::default()
        };
        let b = Input {
            bearing: Some(-179.0),
            ..Default::default()
        };
        assert_eq!(a.interpolate(b, 0.5).bearing, Some(-180.0));
        assert_eq!(Input::from_tilt(Some([0.0; 2]), 0.0).bearing, None);
        assert_eq!(Input::from_tilt(None, 0.0), Input::default());
    }
    #[test]
    fn seeded_pixels_frozen_configuration_cancel_redo_and_spacing_link() {
        use crate::painting::Stroke;
        let mut d = document().unwrap();
        let before = d.snapshot();
        let mut s = style();
        s.dynamics.random_minimum = 0.1;
        s.dynamics.seed = 77;
        s.shape.aspect = 1.0;
        s.spacing = 2.0;
        let paint = |d: &crate::document::Document, s| {
            let mut st = Stroke::begin(d, 0, s).unwrap();
            st.sample(d, [4.5, 12.5]).unwrap();
            st.sample(d, [42.5, 12.5]).unwrap();
            st
        };
        drop(paint(&d, s));
        assert!(d.snapshot() == before);
        paint(&d, s).commit(&mut d).unwrap();
        let after = d.snapshot();
        assert!(d.undo());
        paint(&d, s).commit(&mut d).unwrap();
        assert!(d.snapshot() == after);
        assert!(d.undo());
        s.dynamics.spacing_with_size = false;
        paint(&d, s).commit(&mut d).unwrap();
        assert!(d.snapshot() != after);
        assert_eq!(d.history_counts(), (1, 0));
        assert!(d.undo());
        assert!(d.snapshot() == before);
        assert!(d.redo());
    }
    #[test]
    fn preset_native_reopen_legacy_defaults_and_nonfinite_rejection() {
        use crate::{
            assets::BrushTip,
            brush_presets::Preset,
            mouse_pressure::{Replay, Sample},
            workspaces::Snapshot,
        };
        let root = crate::test_support::TempDir::new();
        let mut snap = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        snap.input.dynamics[0] = Settings {
            direction: Direction::Pen,
            minimum_size: 1.0,
            maximum_size: 12.0,
            random_minimum: 0.3,
            seed: 51,
            pressure: Curve {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let p = Preset::capture(&snap, BrushTip::Pen, "Size dynamics").unwrap();
        let path = root.0.join("dynamics.json");
        p.save(&path, false).unwrap();
        let p = Preset::read(&path).unwrap();
        let applied = p
            .apply(&Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(applied.input.dynamics[0], snap.input.dynamics[0]);
        let mut d = document().unwrap();
        let mut s = style();
        s.dynamics = p.dynamics;
        let mut r = Replay::begin(&d, 0, s, Default::default())
            .unwrap()
            .configure_input(Default::default(), true)
            .unwrap();
        for (n, point) in [[4.5, 12.5], [40.5, 12.5]].into_iter().enumerate() {
            r.push_dynamic(
                Sample {
                    point,
                    micros: Some(n as u64 * 10000),
                },
                0.5,
                0.0,
                Input {
                    tilt: Some(45.0),
                    bearing: Some(60.0),
                    speed: None,
                },
            )
            .unwrap();
        }
        r.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, r.metadata())
            .unwrap();
        let before = d.snapshot();
        let path = root.0.join("dynamics.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let next = crate::files::open(&path, 65536).unwrap();
        assert!(next.snapshot() == before);
        assert_eq!(next.scene.mouse_strokes[0].style.dynamics, s.dynamics);
        let mut old = serde_json::to_value(s).unwrap();
        old.as_object_mut().unwrap().remove("dynamics");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .dynamics
                .is_default()
        );
        let mut old = serde_json::to_value(snap.input).unwrap();
        old.as_object_mut().unwrap().remove("dynamics");
        assert_eq!(
            serde_json::from_value::<crate::tool_input::Settings>(old)
                .unwrap()
                .dynamics,
            [Settings::default(); 7]
        );
        let mut st = crate::painting::Stroke::begin(&d, 0, s).unwrap();
        let old = st.shape();
        assert!(
            st.set_dynamic_input(Input {
                tilt: Some(f64::NAN),
                ..Default::default()
            })
            .is_err()
        );
        assert_eq!(st.shape(), old);
    }
}

#[cfg(test)]
mod scatter_tests {
    use super::*;
    #[test]
    fn circular_particles_face_center_independently_from_scatter_rotation() {
        let d = document().unwrap();
        let mut s = style();
        s.dynamics.direction = Direction::ScatterCenter;
        s.dynamics.scatter_angle = 31.0;
        let mut st = crate::painting::Stroke::begin(&d, 0, s).unwrap();
        for (point, expected) in [
            ([30.5, 12.5], 180.0),
            ([24.5, 18.5], -90.0),
            ([18.5, 12.5], 0.0),
            ([24.5, 6.5], 90.0),
        ] {
            st.direct_dab(&d, point, [24.5, 12.5], 1.0).unwrap();
            assert_eq!(st.shape().angle, expected);
        }
    }
}
