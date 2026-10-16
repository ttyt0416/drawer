//! A local source point resolves to an integer source offset for one pending stroke.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub origin: Option<[f64; 2]>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub enabled: bool,
    pub offset: [i64; 2],
}
impl Settings {
    pub fn valid(self) -> bool {
        self.origin
            .is_none_or(|p| p.iter().all(|n| n.is_finite() && n.abs() <= 1e12))
    }
    pub fn resolve(self, anchor: [f64; 2]) -> Result<Stamp, String> {
        if !self.valid() || anchor.iter().any(|n| !n.is_finite() || n.abs() > 1e12) {
            return Err("Invalid clone source/anchor".into());
        }
        if !self.enabled {
            return Ok(Stamp::default());
        }
        let origin = self
            .origin
            .ok_or("Set a clone source point before painting")?;
        Ok(Stamp {
            enabled: true,
            offset: std::array::from_fn(|n| origin[n].floor() as i64 - anchor[n].floor() as i64),
        })
    }
}
impl Stamp {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(self) -> bool {
        self.offset
            .iter()
            .all(|n| n.unsigned_abs() <= 2_000_000_000_000)
    }
    /// Read the unchanged layer, never the pending stroke's candidate pixels.
    pub fn sample(self, doc: &crate::document::Document, layer: usize, point: [i64; 2]) -> [u8; 4] {
        let Some(x) = point[0].checked_add(self.offset[0]) else {
            return [0; 4];
        };
        let Some(y) = point[1].checked_add(self.offset[1]) else {
            return [0; 4];
        };
        if x < 0 || y < 0 || x >= doc.width as i64 || y >= doc.height as i64 {
            return [0; 4];
        }
        doc.layers
            .get(layer)
            .map_or([0; 4], |l| l.pixel(doc.width, doc.height, x, y))
    }
}

fn document() -> Result<crate::document::Document, String> {
    let mut d = crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 64,
            height: 16,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())?;
    for y in 0..16 {
        for x in 0..64 {
            d.layers[0].pixels[y * 64 + x] = [
                x as u8 * 3,
                255 - x as u8 * 3,
                (x ^ y) as u8,
                if x % 7 == 0 { 0 } else { 180 },
            ];
        }
    }
    Ok(d)
}
fn style(stamp: Stamp) -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: stamp,
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
        tip: crate::assets::BrushTip::PixelPen,
        color: [0; 4],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.25,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
pub fn verify() -> Result<(), String> {
    use crate::painting::Stroke;
    let mut d = document()?;
    let before = d.snapshot();
    let config = Settings {
        enabled: true,
        origin: Some([4.5, 8.5]),
    };
    let s = style(config.resolve([12.5, 8.5])?);
    let mut cancelled = Stroke::begin(&d, 0, s)?;
    cancelled.sample(&d, [12.5, 8.5])?;
    cancelled.sample(&d, [52.5, 8.5])?;
    drop(cancelled);
    if d.snapshot() != before {
        return Err("Clone preview/cancel changed original".into());
    }
    let mut st = Stroke::begin(&d, 0, s)?;
    st.sample(&d, [12.5, 8.5])?;
    st.sample(&d, [52.5, 8.5])?;
    st.commit(&mut d)?;
    for y in 0..16 {
        for x in 0..64 {
            let n = y * 64 + x;
            let source = if y == 8 && (12..=52).contains(&x) {
                y * 64 + x - 8
            } else {
                n
            };
            if d.layers[0].pixels[n] != before.layers[0].pixels[source] {
                return Err(format!("Overlapping clone fed back pending ink at {x},{y}"));
            }
        }
    }
    let after = d.snapshot();
    if d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Clone undo/redo diverged".into());
    }
    let mut d = document()?;
    let mut st = Stroke::begin(
        &d,
        0,
        style(
            Settings {
                enabled: true,
                origin: Some([-20.0, 8.5]),
            }
            .resolve([12.5, 8.5])?,
        ),
    )?;
    st.sample(&d, [12.5, 8.5])?;
    st.sample(&d, [20.5, 8.5])?;
    st.commit(&mut d)?;
    if (12..=20).any(|x| d.layers[0].pixels[8 * 64 + x] != [0; 4]) {
        return Err("Outside-canvas clone source was not transparent".into());
    }
    if (Settings {
        enabled: true,
        origin: None,
    })
    .resolve([12.5, 8.5])
    .is_ok()
    {
        return Err("Unset clone source was accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlap_reads_start_snapshot_rgba_transparency_cancel_and_single_history() {
        verify().unwrap();
    }
    #[test]
    fn clone_uses_mask_ignores_foreground_pigment_and_respects_opacity_alpha_lock_selection() {
        use crate::{assets::BrushTip, painting::Stroke, scene::Rect};
        for tip in [BrushTip::Pen, BrushTip::Eraser, BrushTip::Smudge] {
            let mut d = document().unwrap();
            d.layers[0].pixels[8 * 64 + 4] = [255, 0, 0, 255];
            d.layers[0].pixels[8 * 64 + 12] = [0, 0, 255, 255];
            let mut s = style(
                Settings {
                    enabled: true,
                    origin: Some([4.5, 8.5]),
                }
                .resolve([12.5, 8.5])
                .unwrap(),
            );
            s.tip = tip;
            s.opacity = 0.5;
            s.material.mode = crate::brush_material::Mode::Water;
            s.material.water = 1.0;
            let mut st = Stroke::begin(&d, 0, s).unwrap();
            st.material(&d, crate::mouse_pressure::Effect::Wet, 1.0, [12.5, 8.5])
                .unwrap();
            for _ in 0..8 {
                st.direct_dab(&d, [12.5, 8.5], [12.5, 8.5], 1.0).unwrap();
            }
            st.commit(&mut d).unwrap();
            assert_eq!(
                d.layers[0].pixels[8 * 64 + 12],
                [128, 0, 128, 255],
                "{tip:?}"
            );
        }
        let mut d = document().unwrap();
        d.layers[0].pixels[8 * 64 + 4] = [200, 100, 20, 0];
        let before = d.snapshot();
        let mut s = style(
            Settings {
                enabled: true,
                origin: Some([4.5, 8.5]),
            }
            .resolve([12.5, 8.5])
            .unwrap(),
        );
        s.material.alpha_lock = true;
        let mut st = Stroke::begin(&d, 0, s).unwrap();
        st.direct_dab(&d, [12.5, 8.5], [12.5, 8.5], 1.0).unwrap();
        assert!(!st.commit(&mut d).unwrap());
        assert!(d.snapshot() == before);
        d.scene.selection = Some(Rect {
            min: [12.0, 8.0],
            max: [13.0, 9.0],
        });
        d.layers[0].pixels[8 * 64 + 4] = [200, 100, 20, 255];
        let original_alpha = d.layers[0].pixels[8 * 64 + 12][3];
        let mut st = Stroke::begin(&d, 0, s).unwrap();
        st.sample(&d, [12.5, 8.5]).unwrap();
        st.sample(&d, [20.5, 8.5]).unwrap();
        st.commit(&mut d).unwrap();
        assert_eq!(
            d.layers[0].pixels[8 * 64 + 12],
            [200, 100, 20, original_alpha]
        );
        assert_eq!(
            d.layers[0].pixels[8 * 64 + 13],
            before.layers[0].pixels[8 * 64 + 13]
        );
    }
    #[test]
    fn source_bounds_placed_layers_fractional_grid_and_invalid_configuration() {
        let s = Settings {
            enabled: true,
            origin: Some([4.9, 8.1]),
        }
        .resolve([12.1, 8.9])
        .unwrap();
        assert_eq!(s.offset, [-8, 0]);
        assert!(
            !Settings {
                enabled: true,
                origin: Some([f64::NAN, 0.0])
            }
            .valid()
        );
        assert!(
            !Stamp {
                offset: [i64::MIN, 0],
                enabled: true
            }
            .valid()
        );
        let mut d = document().unwrap();
        d.layers[0].placement = Some(crate::document::PlacedRaster {
            width: 4,
            height: 2,
            offset: [10, 5],
        });
        d.layers[0].pixels = vec![[30, 80, 140, 150]; 8];
        assert_eq!(
            Stamp {
                enabled: true,
                offset: [10, 5]
            }
            .sample(&d, 0, [0, 0]),
            [30, 80, 140, 150]
        );
        assert_eq!(
            Stamp {
                enabled: true,
                offset: [-1, 0]
            }
            .sample(&d, 0, [0, 0]),
            [0; 4]
        );
        assert_eq!(
            Stamp {
                enabled: true,
                offset: [64, 0]
            }
            .sample(&d, 0, [0, 0]),
            [0; 4]
        );
    }
    #[test]
    fn native_style_workspace_state_legacy_defaults_and_frozen_origin() {
        use crate::mouse_pressure::{Replay, Sample};
        let root = crate::test_support::TempDir::new();
        let mut settings = Settings {
            enabled: true,
            origin: Some([4.5, 8.5]),
        };
        let s = style(settings.resolve([12.5, 8.5]).unwrap());
        settings.origin = Some([30.5, 8.5]);
        let mut d = document().unwrap();
        let mut r = Replay::begin(
            &d,
            0,
            s,
            crate::mouse_pressure::Settings {
                effect: crate::mouse_pressure::Effect::Wet,
                ..Default::default()
            },
        )
        .unwrap()
        .configure_input(Default::default(), true)
        .unwrap();
        for point in [[12.5, 8.5], [30.5, 8.5]] {
            r.push_input(
                Sample {
                    point,
                    micros: None,
                },
                1.0,
                0.0,
            )
            .unwrap();
        }
        let original = d.layers[0].pixels[8 * 64 + 4];
        r.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, r.metadata())
            .unwrap();
        assert_eq!(d.layers[0].pixels[8 * 64 + 12], original);
        let before = d.snapshot();
        let path = root.0.join("clone.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let next = crate::files::open(&path, 65536).unwrap();
        assert!(next.snapshot() == before);
        assert_eq!(next.scene.mouse_strokes[0].style.clone_stamp, s.clone_stamp);
        let mut snapshot = crate::workspaces::Snapshot::default();
        snapshot.input.clone_stamp = settings;
        let path = root.0.join("settings.json");
        crate::workspaces::write_json(&path, &snapshot).unwrap();
        assert_eq!(
            crate::workspaces::Snapshot::read(&path)
                .unwrap()
                .input
                .clone_stamp,
            settings
        );
        let mut old = serde_json::to_value(s).unwrap();
        old.as_object_mut().unwrap().remove("clone_stamp");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .clone_stamp
                .is_default()
        );
        let mut old = serde_json::to_value(snapshot.input).unwrap();
        old.as_object_mut().unwrap().remove("clone_stamp");
        assert_eq!(
            serde_json::from_value::<crate::tool_input::Settings>(old)
                .unwrap()
                .clone_stamp,
            Settings::default()
        );
    }
}
