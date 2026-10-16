use crate::scene::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
impl crate::scene::Scene {
    pub fn selection_coverage(&self, point: Point) -> u8 {
        let p = self.transform.inverse().map([
            point[0] - self.selection_translation[0],
            point[1] - self.selection_translation[1],
        ]);
        if let Some(mask) = &self.selection_mask {
            mask.coverage(p)
        } else if let Some(r) = self.selection {
            if p[0] >= r.min[0] && p[0] < r.max[0] && p[1] >= r.min[1] && p[1] < r.max[1] {
                255
            } else {
                0
            }
        } else {
            255
        }
    }
    pub fn selection_empty(&self) -> bool {
        self.selection_mask
            .as_ref()
            .map_or(self.selection.is_none(), |m| m.runs.is_empty())
    }
    pub fn selection_point(&self, point: Point) -> Point {
        let p = self.transform.map(point);
        [
            p[0] + self.selection_translation[0],
            p[1] + self.selection_translation[1],
        ]
    }
}

/// Non-overlapping horizontal coverage runs in the scene's original pixel coordinates.
/// None means unrestricted selection; Some with no runs is an explicit empty selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub runs: Vec<Run>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub x: i32,
    pub y: i32,
    pub length: u32,
    pub alpha: u8,
}
impl Mask {
    pub fn valid(&self) -> bool {
        self.runs.len() <= 32768
            && self.runs.iter().all(|r| {
                r.x.abs_diff(0) <= 1_000_000
                    && r.y.abs_diff(0) <= 1_000_000
                    && (1..=32768).contains(&r.length)
                    && r.alpha > 0
            })
            && self.runs.windows(2).all(|r| {
                r[0].y < r[1].y
                    || (r[0].y == r[1].y && r[0].x as i64 + r[0].length as i64 <= r[1].x as i64)
            })
    }
    pub fn coverage(&self, point: Point) -> u8 {
        if point.iter().any(|n| !n.is_finite()) {
            return 0;
        }
        let y = point[1].floor();
        let x = point[0];
        let n = self.runs.partition_point(|r| {
            (r.y as f64) < y || (r.y as f64 == y && r.x as f64 + r.length as f64 <= x)
        });
        self.runs
            .get(n)
            .filter(|r| r.y as f64 == y && x >= r.x as f64)
            .map_or(0, |r| r.alpha)
    }

    pub fn boundaries(&self) -> Vec<[Point; 2]> {
        let mut rows: BTreeMap<i32, Vec<Run>> = BTreeMap::new();
        for r in &self.runs {
            rows.entry(r.y).or_default().push(*r);
        }
        let mut edges = Vec::new();
        for (y, row) in &rows {
            for (n, r) in row.iter().enumerate() {
                let x = r.x as f64;
                let end = x + r.length as f64;
                let top = *y as f64;
                if n == 0 || row[n - 1].x as f64 + (row[n - 1].length as f64) < x {
                    edges.push([[x, top], [x, top + 1.0]]);
                }
                if n + 1 == row.len() || end < row[n + 1].x as f64 {
                    edges.push([[end, top], [end, top + 1.0]]);
                }
                for (other, boundary) in [(y - 1, top), (y + 1, top + 1.0)] {
                    let mut cursor = x;
                    if let Some(adjacent) = rows.get(&other) {
                        let begin = adjacent.partition_point(|r| r.x as f64 + r.length as f64 <= x);
                        for next in adjacent[begin..].iter().take_while(|r| (r.x as f64) < end) {
                            let left = next.x as f64;
                            if left > cursor {
                                edges.push([[cursor, boundary], [left.min(end), boundary]]);
                            }
                            cursor = cursor.max(left + next.length as f64).min(end);
                        }
                    }
                    if cursor < end {
                        edges.push([[cursor, boundary], [end, boundary]]);
                    }
                }
            }
        }
        edges
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separated_partial_runs_keep_coverage_and_remove_internal_outline_edges() {
        let mask = Mask {
            runs: vec![
                Run {
                    x: 1,
                    y: 1,
                    length: 2,
                    alpha: 128,
                },
                Run {
                    x: 5,
                    y: 1,
                    length: 1,
                    alpha: 255,
                },
                Run {
                    x: 1,
                    y: 2,
                    length: 2,
                    alpha: 64,
                },
            ],
        };
        assert!(mask.valid());
        assert_eq!(mask.coverage([1.5, 1.5]), 128);
        assert_eq!(mask.coverage([1.5, 2.5]), 64);
        assert_eq!(mask.coverage([3.0, 1.0]), 0);
        assert_eq!(mask.coverage([5.5, 1.5]), 255);
        assert!(!mask.boundaries().contains(&[[1.0, 2.0], [3.0, 2.0]]));
        assert_eq!(Mask::default().coverage([0.0; 2]), 0);
        assert!(
            !Mask {
                runs: vec![
                    Run {
                        x: 0,
                        y: 0,
                        length: 3,
                        alpha: 255
                    },
                    Run {
                        x: 2,
                        y: 0,
                        length: 1,
                        alpha: 128
                    }
                ]
            }
            .valid()
        );
        assert!(
            !Mask {
                runs: vec![Run {
                    x: 0,
                    y: 0,
                    length: 0,
                    alpha: 255
                }]
            }
            .valid()
        );
    }
}
