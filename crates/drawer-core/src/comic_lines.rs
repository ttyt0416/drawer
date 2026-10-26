//! Bounded line index: dense radial centers do not force a full scan elsewhere.
use super::{Point, Rect};
#[derive(Clone)]
pub(super) struct Grid {
    rect: Rect,
    bins: Vec<Vec<usize>>,
}
fn crosses(a: Point, b: Point, min: Point, max: Point) -> bool {
    let mut low: f64 = 0.;
    let mut high: f64 = 1.;
    for axis in 0..2 {
        let v = b[axis] - a[axis];
        if v.abs() < 1e-12 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return false;
            }
        } else {
            let t1 = (min[axis] - a[axis]) / v;
            let t2 = (max[axis] - a[axis]) / v;
            low = low.max(t1.min(t2));
            high = high.min(t1.max(t2));
        }
    }
    low <= high
}
impl Grid {
    pub fn new(rect: Rect, lines: &[(Point, Point, f64)]) -> Self {
        let mut bins = vec![vec![]; 1024];
        let step = [
            (rect.max[0] - rect.min[0]) / 32.,
            (rect.max[1] - rect.min[1]) / 32.,
        ];
        for (n, (a, b, w)) in lines.iter().enumerate() {
            for y in 0..32 {
                for x in 0..32 {
                    let min = [
                        rect.min[0] + x as f64 * step[0] - w / 2.,
                        rect.min[1] + y as f64 * step[1] - w / 2.,
                    ];
                    let max = [min[0] + step[0] + w, min[1] + step[1] + w];
                    if crosses(*a, *b, min, max) {
                        bins[y * 32 + x].push(n);
                    }
                }
            }
        }
        Self { rect, bins }
    }
    pub fn candidates(&self, p: Point) -> &[usize] {
        let q = [0, 1].map(|n| {
            (((p[n] - self.rect.min[n]) / (self.rect.max[n] - self.rect.min[n]) * 32.).floor()
                as usize)
                .min(31)
        });
        &self.bins[q[1] * 32 + q[0]]
    }
}
