use serde::{Deserialize, Serialize};
use std::{io, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub minimum: f64,
    pub maximum: f64,
    /// Output at fixed input knots 0, 0.25, 0.5, 0.75, 1.
    pub curve: [f64; 5],
}
impl Default for Response {
    fn default() -> Self {
        Self {
            minimum: 0.0,
            maximum: 1.0,
            curve: [0.0, 0.25, 0.5, 0.75, 1.0],
        }
    }
}
impl Response {
    pub fn error(self) -> Option<&'static str> {
        if !(0.0..=1.0).contains(&self.minimum)
            || !(0.0..=1.0).contains(&self.maximum)
            || self.minimum >= self.maximum
        {
            return Some("Pressure minimum must be below maximum, both in 0–1");
        }
        if self.curve[0] != 0.0
            || self.curve[4] != 1.0
            || self.curve.iter().any(|v| !(0.0..=1.0).contains(v))
            || self.curve.windows(2).any(|v| v[0] > v[1])
        {
            return Some("Curve must rise monotonically from 0 to 1");
        }
        None
    }
    pub fn map(self, pressure: f64) -> Result<f64, String> {
        if let Some(e) = self.error() {
            return Err(e.into());
        }
        if !pressure.is_finite() {
            return Err("Pressure must be finite".into());
        }
        let x = ((pressure - self.minimum) / (self.maximum - self.minimum)).clamp(0.0, 1.0) * 4.0;
        let n = (x.floor() as usize).min(3);
        Ok((self.curve[n] + (self.curve[n + 1] - self.curve[n]) * (x - n as f64)).clamp(0.0, 1.0))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub size: bool,
    pub opacity: bool,
    #[serde(default)]
    pub response: Response,
}
impl Default for Mapping {
    fn default() -> Self {
        Self {
            size: true,
            opacity: false,
            response: Response::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    #[serde(deserialize_with = "crate::brush_slots::read")]
    pub tools: [Mapping; 7],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            tools: [Mapping::default(); 7],
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1 && self.tools.iter().all(|m| m.response.error().is_none())
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if std::fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Pressure preset exceeds 64 KiB"));
        }
        let s: Self = serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        if !s.valid() {
            return Err(io::Error::other("Invalid pressure preset"));
        }
        Ok(s)
    }
    pub fn save(self, path: &Path) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid pressure preset"));
        }
        crate::workspaces::write_json(path, &self)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monotone_curve_range_clamps_output_and_rejects_invalid_configuration() {
        let r = Response {
            minimum: 0.25,
            maximum: 0.75,
            curve: [0.0, 0.1, 0.25, 0.8, 1.0],
        };
        assert_eq!(r.map(-10.0).unwrap(), 0.0);
        assert_eq!(r.map(20.0).unwrap(), 1.0);
        assert_eq!(r.map(0.5).unwrap(), 0.25);
        let mut before = 0.0;
        for n in -100..=1100 {
            let p = r.map(n as f64 / 1000.0).unwrap();
            assert!((0.0..=1.0).contains(&p));
            assert!(p >= before);
            before = p;
        }
        for (minimum, maximum) in [
            (0.5, 0.5),
            (0.7, 0.4),
            (-0.1, 1.0),
            (0.0, 1.1),
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
        ] {
            let bad = Response {
                minimum,
                maximum,
                ..r
            };
            assert!(bad.map(0.5).is_err());
        }
        for curve in [
            [0.0, 0.7, 0.2, 0.8, 1.0],
            [0.0, f64::NAN, 0.5, 0.8, 1.0],
            [0.1, 0.2, 0.5, 0.8, 1.0],
        ] {
            assert!(Response { curve, ..r }.map(0.5).is_err());
        }
        let flat = Response {
            curve: [0.0, 0.4, 0.4, 0.4, 1.0],
            ..Default::default()
        };
        assert!(flat.error().is_none());
        assert!(r.map(f64::INFINITY).is_err());
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        for tool in old["tools"].as_array_mut().unwrap() {
            tool.as_object_mut().unwrap().remove("response");
        }
        let restored: Settings = serde_json::from_value(old).unwrap();
        assert_eq!(restored, Settings::default());
    }
    #[test]
    fn legacy_workspace_defaults_and_import_scope_preserve_pressure() {
        use crate::workspaces::{Scope, Snapshot};
        let mut value = serde_json::to_value(Snapshot::default()).unwrap();
        value.as_object_mut().unwrap().remove("pressure");
        let legacy: Snapshot = serde_json::from_value(value).unwrap();
        assert!(legacy.valid());
        assert_eq!(legacy.pressure, Settings::default());
        let mut incoming = legacy.clone();
        incoming.pressure.tools[2].opacity = true;
        let (layout, _) = legacy
            .merge(
                &incoming,
                Scope {
                    layout: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(layout.pressure, legacy.pressure);
        let (other, _) = legacy
            .merge(
                &incoming,
                Scope {
                    other: true,
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(other.pressure, incoming.pressure);
    }
}
