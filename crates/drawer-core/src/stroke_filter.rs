//! Independent event-based coordinate and device-pressure smoothing.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub position_enabled: bool,
    pub pressure_enabled: bool,
    pub pressure_strength: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            position_enabled: true,
            pressure_enabled: false,
            pressure_strength: 0.5,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (0.0..=0.95).contains(&self.pressure_strength)
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn position(self, strength: f64) -> f64 {
        if self.position_enabled { strength } else { 0.0 }
    }
}
#[derive(Clone, Copy, Default)]
pub struct Pressure {
    value: Option<f64>,
}
impl Pressure {
    pub fn receive(&mut self, settings: Settings, raw: f64) -> Result<f64, String> {
        if !settings.valid() || !(0.0..=1.0).contains(&raw) {
            return Err("Invalid pressure filter or normalized input".into());
        }
        let value = if settings.pressure_enabled {
            self.value.map_or(raw, |previous| {
                raw + (previous - raw) * settings.pressure_strength
            })
        } else {
            raw
        };
        self.value = Some(value);
        Ok(value)
    }
    pub fn value(self) -> Option<f64> {
        self.value
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
            size: 10.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [40, 100, 180, 255],
        smoothing: 0.8,
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        pressure: crate::pressure::Mapping {
            size: true,
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
        8192,
    )
    .map_err(|e| e.to_string())
}
pub fn verify() -> Result<(), String> {
    use crate::painting::Stroke;
    let d = document()?;
    let mut settings = style();
    settings.filters.pressure_enabled = true;
    settings.filters.pressure_strength = 0.75;
    let mut stroke = Stroke::begin(&d, 0, settings)?;
    for (p, q) in [(0.2, 0.2), (0.8, 0.35), (0.2, 0.3125)] {
        stroke.sample_pressure(&d, [12.5, 8.5], p)?;
        if (stroke.shape().size - 10.0 * q).abs() > 1e-10
            || stroke.trace().corrected != Some([12.5, 8.5])
        {
            return Err("Pressure smoothing moved the position or filtered more than once".into());
        }
    }
    let mut raw = style();
    raw.filters.position_enabled = false;
    raw.filters.pressure_enabled = true;
    let mut stroke = Stroke::begin(&d, 0, raw)?;
    for point in [[4.5, 8.5], [18.5, 4.5], [20.5, 12.5]] {
        stroke.sample_pressure(&d, point, 0.5)?;
        if stroke.trace().corrected != Some(point) || (stroke.shape().size - 5.0).abs() > 1e-10 {
            return Err("Independent position-off/constant-pressure path diverged".into());
        }
    }
    let mut filtered = style();
    filtered.filters.pressure_enabled = true;
    filtered.filters.pressure_strength = 0.75;
    let mut expected = filtered;
    expected.filters.pressure_enabled = false;
    let render = |style: crate::painting::Style,
                  pressures: [f64; 3],
                  clock: bool|
     -> Result<Vec<[u8; 4]>, String> {
        let mut doc = document()?;
        let mut replay = crate::mouse_pressure::Replay::begin(&doc, 0, style, Default::default())?
            .configure_input(Default::default(), true)?;
        for (n, (point, p)) in [[4.5, 8.5], [12.5, 8.5], [24.5, 8.5]]
            .into_iter()
            .zip(pressures)
            .enumerate()
        {
            replay.push_input(
                crate::mouse_pressure::Sample {
                    point,
                    micros: Some(n as u64 * 1000),
                },
                p,
                0.0,
            )?;
            if n == 1 && clock {
                for tick in 1..=10 {
                    replay.push_clock(crate::mouse_pressure::Sample {
                        point,
                        micros: Some(1000 + tick),
                    })?;
                }
            }
        }
        replay.render(&doc, true)?.commit(&mut doc)?;
        Ok(doc.layers.remove(0).pixels)
    };
    let a = render(filtered, [0.2, 0.8, 0.2], false)?;
    let b = render(expected, [0.2, 0.35, 0.3125], false)?;
    let c = render(filtered, [0.2, 0.8, 0.2], true)?;
    if a != b || a != c {
        return Err(
            "Replay applied the pressure filter to interpolated dabs or clock ticks".into(),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn position_pressure_jitter_are_independent_and_replay_uses_one_filter_per_event() {
        verify().unwrap();
    }
    #[test]
    fn constant_pressure_position_filter_does_not_change_pressure_or_pressure_only_positions() {
        let d = document().unwrap();
        for position in [false, true] {
            let mut pixels = None;
            for pressure in [false, true] {
                let mut s = style();
                s.filters.position_enabled = position;
                s.filters.pressure_enabled = pressure;
                s.filters.pressure_strength = 0.95;
                let mut stroke = crate::painting::Stroke::begin(&d, 0, s).unwrap();
                for point in [[2.5, 8.5], [10.5, 3.5], [18.5, 12.5], [24.5, 8.5]] {
                    stroke.sample_pressure(&d, point, 0.5).unwrap();
                    assert!((stroke.shape().size - 5.0).abs() < 1e-10);
                }
                let trace = stroke.trace();
                assert_eq!(trace.raw, Some([24.5, 8.5]));
                let mut result = Vec::new();
                for y in 0..16 {
                    for x in 0..32 {
                        result.push(stroke.pixel(&d, 0, &d.layers[0], x, y).unwrap_or([0; 4]));
                    }
                }
                if let Some(before) = &pixels {
                    assert_eq!(&result, before);
                } else {
                    pixels = Some(result);
                }
            }
        }
    }
    #[test]
    fn invalid_pressure_and_legacy_settings_do_not_advance_state_or_replace_curves() {
        let s = Settings {
            pressure_enabled: true,
            ..Default::default()
        };
        let mut p = Pressure::default();
        assert_eq!(p.receive(s, 0.4).unwrap(), 0.4);
        for input in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(p.receive(s, input).is_err());
            assert_eq!(p.value(), Some(0.4));
        }
        assert!(
            !Settings {
                pressure_strength: 0.951,
                ..s
            }
            .valid()
        );
        assert!(
            !Settings {
                pressure_strength: f64::NAN,
                ..s
            }
            .valid()
        );
        let mut old = serde_json::to_value(crate::tool_input::Settings::default()).unwrap();
        old.as_object_mut().unwrap().remove("filters");
        let old: crate::tool_input::Settings = serde_json::from_value(old).unwrap();
        assert!(old.filters.iter().all(Settings::is_default));
        let mut old = serde_json::to_value(style()).unwrap();
        old.as_object_mut().unwrap().remove("filters");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .filters
                .is_default()
        );
    }
    #[test]
    fn filters_preset_and_native_style_roundtrip_keep_original_pressure_mapping() {
        let root = crate::test_support::TempDir::new();
        let mut s = crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        let f = Settings {
            position_enabled: false,
            pressure_enabled: true,
            pressure_strength: 0.9,
        };
        s.input.filters[0] = f;
        s.input.smoothing[0] = 0.8;
        s.pressure.tools[0].opacity = true;
        s.pressure.tools[0].response.curve = [0.0, 0.1, 0.35, 0.6, 1.0];
        let mapping = s.pressure.tools[0];
        let p =
            crate::brush_presets::Preset::capture(&s, crate::assets::BrushTip::Pen, "Filtered pen")
                .unwrap();
        let p = crate::brush_presets::Preset::decode(&p.encode().unwrap()).unwrap();
        let restored = p
            .apply(&crate::workspaces::Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(restored.input.filters[0], f);
        assert_eq!(restored.pressure.tools[0], mapping);
        let mut d = document().unwrap();
        let mut style = style();
        style.filters = f;
        style.pressure = mapping;
        let mut replay = crate::mouse_pressure::Replay::begin(&d, 0, style, Default::default())
            .unwrap()
            .configure_input(Default::default(), true)
            .unwrap();
        for (point, p) in [([4.5, 8.5], 0.2), ([24.5, 8.5], 0.8)] {
            replay
                .push_input(
                    crate::mouse_pressure::Sample {
                        point,
                        micros: None,
                    },
                    p,
                    0.0,
                )
                .unwrap();
        }
        let meta = replay.metadata();
        replay
            .render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, meta)
            .unwrap();
        let before = d.snapshot();
        let path = root.0.join("filter.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let opened = crate::files::open(&path, 1000000).unwrap();
        assert!(opened.snapshot() == before);
        assert_eq!(opened.scene.mouse_strokes[0].style.filters, f);
        assert_eq!(opened.scene.mouse_strokes[0].style.pressure, mapping);
    }
}
