use crate::{scene::Point, view::View};
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub size: f64,
    pub aspect: f64,
    pub angle: f64,
}
impl Default for Shape {
    fn default() -> Self {
        Self {
            size: 12.0,
            aspect: 1.0,
            angle: 0.0,
        }
    }
}
impl Shape {
    pub fn valid(self) -> bool {
        self.size.is_finite()
            && self.size > 0.0
            && self.size <= 4096.0
            && (0.05..=1.0).contains(&self.aspect)
            && self.angle.is_finite()
    }
    pub fn outline(
        self,
        view: View,
        point: Point,
        size: Point,
        center: Point,
        ppp: f64,
    ) -> Vec<Point> {
        if !self.valid() || !ppp.is_finite() || ppp <= 0.0 || point.iter().any(|n| !n.is_finite()) {
            return Vec::new();
        }
        let (sin, cos) = self.angle.to_radians().sin_cos();
        (0..64)
            .map(|n| {
                let angle = n as f64 * std::f64::consts::TAU / 64.0;
                let x = angle.cos() * self.size / 2.0;
                let y = angle.sin() * self.size * self.aspect / 2.0;
                view.document_to_screen(
                    [point[0] + x * cos - y * sin, point[1] + x * sin + y * cos],
                    size,
                    center,
                    ppp,
                )
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_large_rotated_elliptic_footprints_match_first_input_across_mirror_dpi_zoom() {
        let size = [800.0, 600.0];
        let center = [550.0, 400.0];
        let screen = [387.0, 322.0];
        for diameter in [1.0, 4096.0] {
            for ppp in [1.0, 1.5, 2.0, 4.0] {
                for zoom in [0.01, 1.0, 64.0] {
                    for flipped in [false, true] {
                        let view = View {
                            zoom,
                            angle: 37.0,
                            flipped,
                            pan: [10.0, -20.0],
                        };
                        let point = view.screen_to_document(screen, size, center, ppp);
                        let shape = Shape {
                            size: diameter,
                            aspect: 0.3,
                            angle: 29.0,
                        };
                        let outline = shape.outline(view, point, size, center, ppp);
                        for n in 0..32 {
                            let midpoint = [
                                (outline[n][0] + outline[n + 32][0]) / 2.0,
                                (outline[n][1] + outline[n + 32][1]) / 2.0,
                            ];
                            assert!(
                                (midpoint[0] - screen[0]).abs() < 1e-6
                                    && (midpoint[1] - screen[1]).abs() < 1e-6
                            );
                        }
                        let a = view.screen_to_document(outline[0], size, center, ppp);
                        let b = view.screen_to_document(outline[16], size, center, ppp);
                        assert!(
                            ((a[0] - point[0]).hypot(a[1] - point[1]) - diameter / 2.0).abs()
                                < 1e-6
                        );
                        assert!(
                            ((b[0] - point[0]).hypot(b[1] - point[1]) - diameter * 0.3 / 2.0).abs()
                                < 1e-6
                        );
                    }
                }
            }
        }
    }
}
