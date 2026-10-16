//! Deterministic local brush-tip sequences. Asset 0 uses the current tool recipe.
use crate::assets::BrushTip;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Order {
    #[default]
    Repeat,
    Reverse,
    DoNotRepeat,
    Random,
    OneTimeOnly,
    OneRandomCycle,
}
impl Order {
    pub const ALL: [Self; 6] = [
        Self::Repeat,
        Self::Reverse,
        Self::DoNotRepeat,
        Self::Random,
        Self::OneTimeOnly,
        Self::OneRandomCycle,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Repeat => "Repeat",
            Self::Reverse => "Reverse",
            Self::DoNotRepeat => "Do not repeat",
            Self::Random => "Random",
            Self::OneTimeOnly => "One time only",
            Self::OneRandomCycle => "One random cycle",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub slots: [crate::tip_texture::Slot; 8],
    pub enabled: bool,
    pub count: u8,
    pub tips: [u32; 8],
    pub order: Order,
    pub seed: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            slots: [Default::default(); 8],
            enabled: false,
            count: 1,
            tips: [0; 8],
            order: Order::Repeat,
            seed: 0,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        (1..=8).contains(&self.count) && self.slots.iter().all(|s| s.valid())
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn missing(self) -> Vec<u32> {
        self.tips[..(self.count as usize).min(8)]
            .iter()
            .copied()
            .filter(|n| *n > BrushTip::ALL.len() as u32)
            .collect()
    }
    pub fn tip(self, slot: usize, current: BrushTip) -> BrushTip {
        self.tips
            .get(slot)
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| BrushTip::ALL.get(n as usize))
            .copied()
            .unwrap_or(current)
    }
    pub fn label(source: u32) -> String {
        if source == 0 {
            "Current tool tip".into()
        } else {
            BrushTip::ALL.get(source as usize - 1).map_or_else(
                || format!("Missing tip #{source} — current tip fallback"),
                |t| format!("Owned {} mask", t.label()),
            )
        }
    }
}
#[derive(Clone, Default)]
pub struct Sequence {
    at: u64,
    random: u64,
    permutation: [u8; 8],
    initialized: bool,
    pub last: Option<usize>,
}
impl Sequence {
    fn random(&mut self) -> u64 {
        self.random = self.random.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.random;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    pub fn next(&mut self, s: Settings) -> Option<usize> {
        if !s.enabled {
            return Some(0);
        }
        if !s.valid() {
            return None;
        }
        if !self.initialized {
            self.random = s.seed as u64;
            for (n, v) in self.permutation.iter_mut().enumerate() {
                *v = n as u8;
            }
            for n in (1..s.count as usize).rev() {
                let other = self.random() as usize % (n + 1);
                self.permutation.swap(n, other);
            }
            self.initialized = true;
        }
        let count = s.count as u64;
        let slot = match s.order {
            Order::Repeat => (self.at % count) as usize,
            Order::Reverse => {
                let period = (count.saturating_sub(1) * 2).max(1);
                let n = self.at % period;
                if n < count {
                    n as usize
                } else {
                    (period - n) as usize
                }
            }
            Order::DoNotRepeat => self.at.min(count - 1) as usize,
            Order::Random => (self.random() % count) as usize,
            Order::OneTimeOnly => {
                if self.at >= count {
                    return None;
                }
                self.at as usize
            }
            Order::OneRandomCycle => {
                if self.at >= count {
                    return None;
                }
                self.permutation[self.at as usize] as usize
            }
        };
        self.at = self.at.saturating_add(1);
        self.last = Some(slot);
        Some(slot)
    }
}

/// Verify ordered masks and atomic publication in the installed application too.
pub fn verify() -> Result<(), String> {
    use crate::{
        assets::BrushTip,
        brush_cursor::Shape,
        document::{Background, CanvasSpec, Document},
        painting::{Stroke, Style},
    };
    for (order, expected) in [
        (
            Order::Repeat,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(0),
                Some(1),
                Some(2),
                Some(0),
            ],
        ),
        (
            Order::Reverse,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(1),
                Some(0),
                Some(1),
                Some(2),
            ],
        ),
        (
            Order::DoNotRepeat,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(2),
                Some(2),
                Some(2),
                Some(2),
            ],
        ),
        (
            Order::OneTimeOnly,
            vec![Some(0), Some(1), Some(2), None, None, None, None],
        ),
    ] {
        let s = Settings {
            enabled: true,
            count: 3,
            order,
            ..Default::default()
        };
        let mut q = Sequence::default();
        if (0..7).map(|_| q.next(s)).collect::<Vec<_>>() != expected {
            return Err(format!("Incorrect {} order", order.label()));
        }
    }
    for order in [Order::Random, Order::OneRandomCycle] {
        let s = Settings {
            enabled: true,
            count: 8,
            order,
            seed: 2471,
            ..Default::default()
        };
        let collect = || {
            let mut q = Sequence::default();
            (0..20).map(|_| q.next(s)).collect::<Vec<_>>()
        };
        let a = collect();
        if a != collect() {
            return Err("Seeded tip sequence changed".into());
        }
        if order == Order::OneRandomCycle {
            let mut first = a[..8].to_vec();
            first.sort();
            if first != (0..8).map(Some).collect::<Vec<_>>() || a[8..].iter().any(Option::is_some) {
                return Err("Random cycle repeated or failed to stop".into());
            }
        }
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 48,
            height: 20,
            dpi: 300.0,
            background: Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())?;
    let before = d.snapshot();
    let sequence = Settings {
        slots: Default::default(),
        enabled: true,
        count: 3,
        tips: [7, 3, 999, 0, 0, 0, 0, 0],
        order: Order::OneTimeOnly,
        seed: 42,
    };
    let style = Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence,
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: Shape {
            size: 6.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: BrushTip::Pen,
        color: [30, 90, 200, 255],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    };
    let make = || -> Result<Stroke, String> {
        let mut stroke = Stroke::begin(&d, 0, style)?;
        for (x, tip) in [
            (6.5, BrushTip::PixelPen),
            (17.5, BrushTip::Eraser),
            (28.5, BrushTip::Pen),
        ] {
            stroke.direct_dab(&d, [x, 10.5], [x, 10.5], 1.0)?;
            if stroke.mask_tip() != tip {
                return Err("Actual mask order differs from sequence".into());
            }
        }
        stroke.direct_dab(&d, [40.5, 10.5], [40.5, 10.5], 1.0)?;
        Ok(stroke)
    };
    drop(make()?);
    let stroke = make()?;
    if d.snapshot() != before {
        return Err("Cancelled/preview sequence changed source".into());
    }
    stroke.commit(&mut d)?;
    if [6, 17, 28]
        .iter()
        .any(|x| d.composite_pixel(*x, 10)[3] != 255)
        || d.composite_pixel(40, 10)[3] != 0
    {
        return Err(
            "Mask changed parent behavior, missing fallback, or failed to terminate".into(),
        );
    }
    let after = d.snapshot();
    if d.history_counts() != (1, 0)
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Tip sequence history changed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_orders_actual_masks_missing_fallback_cancel_and_single_history() {
        verify().unwrap();
    }
    #[test]
    fn singleton_limits_and_random_seeds_are_distinct() {
        for order in Order::ALL {
            let mut q = Sequence::default();
            let s = Settings {
                enabled: true,
                order,
                ..Default::default()
            };
            assert_eq!(q.next(s), Some(0));
            assert_eq!(
                q.next(s),
                if matches!(order, Order::OneTimeOnly | Order::OneRandomCycle) {
                    None
                } else {
                    Some(0)
                }
            );
        }
        let collect = |seed| {
            let s = Settings {
                enabled: true,
                count: 8,
                seed,
                order: Order::Random,
                ..Default::default()
            };
            let mut q = Sequence::default();
            (0..30).map(|_| q.next(s)).collect::<Vec<_>>()
        };
        assert_ne!(collect(1), collect(2));
        for count in [0, 9, 255] {
            assert!(
                !Settings {
                    count,
                    ..Default::default()
                }
                .valid()
            );
        }
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn preset_native_legacy_and_deleted_ids_preserve_frozen_settings() {
        use crate::{assets::BrushTip, brush_presets::Preset, workspaces::Snapshot};
        let root = crate::test_support::TempDir::new();
        let mut s = Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        s.input.sequences[0] = Settings {
            slots: Default::default(),
            enabled: true,
            count: 3,
            tips: [7, 2, 8001, 0, 0, 0, 0, 0],
            order: Order::OneRandomCycle,
            seed: 323,
        };
        s.input.materials[0].space = crate::color_mix::Space::Perceptual;
        s.input.materials[0].quality = crate::color_mix::Quality::High;
        s.input.materials[0].mode = crate::brush_material::Mode::RunningColor;
        s.input.ends[0] = crate::stroke_ends::Settings {
            method: crate::stroke_ends::Method::Fade,
            fade_minimum: 0.2,
            start: 0.0,
            end: 10.0,
        };
        let preset = Preset::capture(&s, BrushTip::Pen, "Local sequence").unwrap();
        let path = root.0.join("sequence.json");
        preset.save(&path, false).unwrap();
        let loaded = Preset::read(&path).unwrap();
        let applied = loaded
            .apply(&Snapshot {
                properties: Some(Default::default()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(applied.input.sequences[0], s.input.sequences[0]);
        assert_eq!(applied.input.materials[0], s.input.materials[0]);
        assert_eq!(applied.input.ends[0], s.input.ends[0]);
        assert_eq!(loaded.sequence.missing(), vec![8001]);
        let mut d = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: 48,
                height: 20,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            65536,
        )
        .unwrap();
        let style = crate::painting::Style {
            clone_stamp: Default::default(),
            variation: Default::default(),
            pattern: Default::default(),
            dynamics: Default::default(),
            sequence: loaded.sequence,
            filters: loaded.filters,
            profile: loaded.profile,
            texture: loaded.overlay,
            material: loaded.material,
            shape: crate::brush_cursor::Shape {
                size: 6.0,
                aspect: 1.0,
                angle: 0.0,
            },
            tip: BrushTip::Pen,
            color: [80, 120, 220, 255],
            opacity: 1.0,
            hardness: 1.0,
            spacing: 0.5,
            smoothing: 0.0,
            pressure: Default::default(),
        };
        let mut r = crate::mouse_pressure::Replay::begin(&d, 0, style, Default::default())
            .unwrap()
            .configure_input(loaded.ends, true)
            .unwrap();
        for point in [[4.5, 10.5], [40.5, 10.5]] {
            r.push_input(
                crate::mouse_pressure::Sample {
                    point,
                    micros: None,
                },
                1.0,
                0.0,
            )
            .unwrap();
        }
        r.render(&d, true)
            .unwrap()
            .commit_mouse(&mut d, r.metadata())
            .unwrap();
        let before = d.snapshot();
        let path = root.0.join("sequence.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let next = crate::files::open(&path, 65536).unwrap();
        assert!(next.snapshot() == before);
        assert_eq!(next.scene.mouse_strokes[0].style.sequence, loaded.sequence);
        let mut old = serde_json::to_value(style).unwrap();
        old.as_object_mut().unwrap().remove("sequence");
        assert!(
            serde_json::from_value::<crate::painting::Style>(old)
                .unwrap()
                .sequence
                .is_default()
        );
        let mut old = serde_json::to_value(s.input).unwrap();
        old.as_object_mut().unwrap().remove("sequences");
        assert_eq!(
            serde_json::from_value::<crate::tool_input::Settings>(old)
                .unwrap()
                .sequences,
            [Settings::default(); 7]
        );
    }
}
