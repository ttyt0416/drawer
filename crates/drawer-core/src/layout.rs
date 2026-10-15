//! Workspace layout is stored in physical pixels, independently of drawing undo.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    pub width: bool,
    pub height: bool,
    pub position: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presentation {
    #[default]
    Stack,
    Tabs,
    Popup,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arrangement {
    #[default]
    Tabs,
    Cascade,
    Tile,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub width: [f64; 2],
    pub height: [f64; 2],
    pub dock_locks: [Lock; 2],
    pub panel_locks: [Lock; 4],
    pub presentation: [Presentation; 2],
    pub selected: [usize; 2],
    pub titles: bool,
    pub menu: bool,
    pub arrangement: Arrangement,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            width: [260.0; 2],
            height: [600.0; 2],
            dock_locks: [Lock::default(); 2],
            panel_locks: [Lock::default(); 4],
            presentation: [Presentation::Stack; 2],
            selected: [0; 2],
            titles: true,
            menu: true,
            arrangement: Arrangement::Tabs,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.selected.iter().all(|n| *n < 4)
            && self
                .width
                .iter()
                .chain(&self.height)
                .all(|n| (80.0..=4096.0).contains(n))
    }
    pub fn dock_locked(&self, index: usize) -> bool {
        self.dock_locks[index].position
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub position: [f64; 2],
    pub size: [f64; 2],
}
/// Coordinates are logical UI pixels, while cascade spacing is a physical-pixel measure.
pub fn arrange(mode: Arrangement, count: usize, size: [f64; 2], ppp: f64) -> Vec<Rect> {
    if count == 0
        || count > 256
        || mode == Arrangement::Tabs
        || !ppp.is_finite()
        || ppp <= 0.0
        || !size.iter().all(|n| n.is_finite() && *n > 0.0)
    {
        return Vec::new();
    }
    match mode {
        Arrangement::Tile => {
            let cols = (count as f64).sqrt().ceil() as usize;
            let rows = count.div_ceil(cols);
            (0..count)
                .map(|n| Rect {
                    position: [
                        (n % cols) as f64 * size[0] / cols as f64,
                        (n / cols) as f64 * size[1] / rows as f64,
                    ],
                    size: [size[0] / cols as f64, size[1] / rows as f64],
                })
                .collect()
        }
        Arrangement::Cascade => {
            let step = (24.0 / ppp).min(size[0] / 4.0).min(size[1] / 4.0);
            let span = count.min(8).saturating_sub(1) as f64 * step;
            let extent = [
                (size[0] - span).max(size[0] * 0.6),
                (size[1] - span).max(size[1] * 0.6),
            ];
            (0..count)
                .map(|n| Rect {
                    position: [
                        ((n % 8) as f64 * step).min(size[0] - extent[0]),
                        ((n % 8) as f64 * step).min(size[1] - extent[1]),
                    ],
                    size: extent,
                })
                .collect()
        }
        Arrangement::Tabs => Vec::new(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layouts_stay_inside_small_screens_at_all_scales_and_limits() {
        for ppp in [0.5, 1.0, 1.5, 2.0, 4.0] {
            for count in [1, 2, 3, 7, 256] {
                for mode in [Arrangement::Tile, Arrangement::Cascade] {
                    let r = arrange(mode, count, [640.0 / ppp, 480.0 / ppp], ppp);
                    assert_eq!(r.len(), count);
                    for r in r {
                        for axis in 0..2 {
                            assert!(r.position[axis] >= 0.0 && r.size[axis] > 0.0);
                            assert!(
                                r.position[axis] + r.size[axis]
                                    <= [640.0 / ppp, 480.0 / ppp][axis] + 1e-8
                            );
                        }
                    }
                }
            }
        }
        assert!(arrange(Arrangement::Tile, 0, [1.0, 1.0], 1.0).is_empty());
        assert!(arrange(Arrangement::Tile, 257, [1.0, 1.0], 1.0).is_empty());
        assert!(arrange(Arrangement::Tile, 1, [f64::NAN, 1.0], 1.0).is_empty());
        let mut s = Settings::default();
        assert!(s.valid());
        s.width[0] = 80.0;
        s.height[1] = 4096.0;
        assert!(s.valid());
        s.width[0] = f64::INFINITY;
        assert!(!s.valid());
    }
}
