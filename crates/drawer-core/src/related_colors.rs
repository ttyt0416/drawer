//! Bounded local color grids; view density does not participate in color arithmetic.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub rows: u8,
    pub columns: u8,
    pub tile: u8,
    pub lines: bool,
}
impl Grid {
    pub fn valid(self) -> bool {
        (1..=16).contains(&self.rows)
            && (1..=16).contains(&self.columns)
            && (12..=64).contains(&self.tile)
    }
    pub fn count(self) -> usize {
        self.rows as usize * self.columns as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub visible: bool,
    pub endpoints: [[u8; 4]; 2],
    pub middle: Grid,
    pub similar: Grid,
    pub span: [f64; 2],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            visible: false,
            endpoints: [[0, 0, 0, 255], [255; 4]],
            middle: Grid {
                rows: 1,
                columns: 9,
                tile: 24,
                lines: true,
            },
            similar: Grid {
                rows: 5,
                columns: 5,
                tile: 24,
                lines: true,
            },
            span: [0.25; 2],
        }
    }
}
fn ratio(n: usize, count: usize) -> f64 {
    if count <= 1 {
        0.5
    } else {
        n as f64 / (count - 1) as f64
    }
}
fn interpolate(a: [u8; 4], b: [u8; 4], t: f64) -> [u8; 4] {
    if a == b || t == 0.0 {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    let aa = a[3] as f64 * (1.0 - t);
    let ba = b[3] as f64 * t;
    let alpha = aa + ba;
    let mut out = [0; 4];
    for i in 0..3 {
        out[i] = if alpha == 0.0 {
            (a[i] as f64 * (1.0 - t) + b[i] as f64 * t).round() as u8
        } else {
            ((a[i] as f64 * aa + b[i] as f64 * ba) / alpha).round() as u8
        }
    }
    out[3] = alpha.round() as u8;
    out
}
impl Settings {
    pub fn valid(self) -> bool {
        self.middle.valid()
            && self.similar.valid()
            && self.span.iter().all(|v| (0.0..=1.0).contains(v))
    }
    pub fn middle_colors(self) -> Result<Vec<[u8; 4]>, String> {
        if !self.valid() {
            return Err("Invalid related-color grid".into());
        }
        Ok((0..self.middle.count())
            .map(|i| {
                interpolate(
                    self.endpoints[0],
                    self.endpoints[1],
                    ratio(i, self.middle.count()),
                )
            })
            .collect())
    }
    pub fn similar_colors(self, current: [u8; 4], hue: f64) -> Result<Vec<[u8; 4]>, String> {
        if !self.valid() || !hue.is_finite() || !(0.0..=360.0).contains(&hue) {
            return Err("Invalid similar-color grid or hue".into());
        }
        let [h, s, v] = crate::color_selector::hsv(current, hue);
        let mut out = Vec::with_capacity(self.similar.count());
        for row in 0..self.similar.rows {
            for column in 0..self.similar.columns {
                let s = (s
                    + (ratio(column as usize, self.similar.columns as usize) * 2.0 - 1.0)
                        * self.span[0])
                    .clamp(0.0, 1.0);
                let v = (v
                    + (ratio(row as usize, self.similar.rows as usize) * 2.0 - 1.0) * self.span[1])
                    .clamp(0.0, 1.0);
                out.push(crate::color_selector::from_hsv([h, s, v], current[3])?);
            }
        }
        Ok(out)
    }
}
pub fn verify() -> Result<(), String> {
    let mut s = Settings::default();
    let mid = s.middle_colors()?;
    if mid[0] != [0, 0, 0, 255] || mid[4] != [128, 128, 128, 255] || mid[8] != [255; 4] {
        return Err("Intermediate black/white reference mismatch".into());
    }
    for color in [[12, 34, 56, 0], [0, 0, 0, 128], [255; 4]] {
        s.endpoints = [color; 2];
        if s.middle_colors()?.iter().any(|p| *p != color) {
            return Err("Equal endpoint colors changed".into());
        }
    }
    s.endpoints = [[255, 0, 0, 0], [0, 0, 255, 255]];
    if s.middle_colors()?[4] != [0, 0, 255, 128] {
        return Err("Intermediate premultiplied alpha mismatch".into());
    }
    s = Settings::default();
    let current = [20, 80, 160, 128];
    let base = s.middle_colors()?;
    let near = s.similar_colors(current, 217.0)?;
    s.middle.tile = 64;
    s.middle.lines = false;
    s.similar.tile = 12;
    s.similar.lines = false;
    if s.middle_colors() != Ok(base) || s.similar_colors(current, 217.0) != Ok(near.clone()) {
        return Err("Tile/grid lines changed generated colors".into());
    }
    s.middle.rows = 3;
    s.middle.columns = 3;
    let coarse = s.middle_colors()?;
    s.middle.rows = 5;
    s.middle.columns = 5;
    if coarse[4] != s.middle_colors()?[12] || near[12] != current {
        return Err("Common midpoint/current-color samples changed with density".into());
    }
    s.similar.rows = 1;
    s.similar.columns = 1;
    for c in [
        [0, 0, 0, 0],
        [255, 255, 255, 128],
        [255, 0, 0, 255],
        current,
    ] {
        if s.similar_colors(c, 270.0)? != vec![c] {
            return Err("One-cell similar grid changed the current color".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_samples_density_view_and_alpha() {
        verify().unwrap();
    }
    #[test]
    fn density_common_coordinates_and_extremes() {
        let mut s = Settings::default();
        let color = [120, 40, 220, 0];
        let a = s.similar_colors(color, 0.0).unwrap();
        s.similar.rows = 9;
        s.similar.columns = 9;
        let b = s.similar_colors(color, 0.0).unwrap();
        for row in 0..5 {
            for column in 0..5 {
                assert_eq!(a[row * 5 + column], b[row * 2 * 9 + column * 2]);
            }
        }
        s.span = [1.0; 2];
        assert_eq!(s.similar_colors([0; 4], 360.0).unwrap().len(), 81);
        s.middle.rows = 0;
        assert!(s.middle_colors().is_err());
        s.middle.rows = 16;
        s.similar.columns = 17;
        assert!(!s.valid());
    }
}
