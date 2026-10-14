#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f64,
    pub angle: f64,
    pub pan: [f64; 2],
    pub flipped: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            angle: 0.0,
            pan: [0.0; 2],
            flipped: false,
        }
    }
}
impl View {
    pub fn document_to_screen(
        self,
        point: [f64; 2],
        size: [f64; 2],
        center: [f64; 2],
        pixels_per_point: f64,
    ) -> [f64; 2] {
        let mut x = point[0] - size[0] / 2.0;
        let y = point[1] - size[1] / 2.0;
        if self.flipped {
            x = -x;
        }
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let scale = self.zoom / pixels_per_point;
        [
            center[0] + self.pan[0] + (x * cos - y * sin) * scale,
            center[1] + self.pan[1] + (x * sin + y * cos) * scale,
        ]
    }
    pub fn screen_to_document(
        self,
        point: [f64; 2],
        size: [f64; 2],
        center: [f64; 2],
        pixels_per_point: f64,
    ) -> [f64; 2] {
        let x = (point[0] - center[0] - self.pan[0]) * pixels_per_point / self.zoom;
        let y = (point[1] - center[1] - self.pan[1]) * pixels_per_point / self.zoom;
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let mut rx = x * cos + y * sin;
        let ry = -x * sin + y * cos;
        if self.flipped {
            rx = -rx;
        }
        [rx + size[0] / 2.0, ry + size[1] / 2.0]
    }
    pub fn pan_by(&mut self, delta: [f64; 2]) {
        if delta.iter().all(|value| value.is_finite()) {
            self.pan = [
                (self.pan[0] + delta[0]).clamp(-1e9, 1e9),
                (self.pan[1] + delta[1]).clamp(-1e9, 1e9),
            ];
        }
    }
    pub fn rotate(&mut self, angle: f64) {
        if angle.is_finite() {
            self.angle = angle.rem_euclid(360.0);
        }
    }
    pub fn zoom_about(
        &mut self,
        factor: f64,
        anchor: [f64; 2],
        size: [f64; 2],
        center: [f64; 2],
        pixels_per_point: f64,
    ) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let point = self.screen_to_document(anchor, size, center, pixels_per_point);
        self.zoom = (self.zoom * factor).clamp(0.01, 64.0);
        let after = self.document_to_screen(point, size, center, pixels_per_point);
        self.pan_by([anchor[0] - after[0], anchor[1] - after[1]]);
    }
    pub fn encode(self) -> String {
        format!(
            "1 {} {} {} {} {}\n",
            self.zoom,
            self.angle,
            self.pan[0],
            self.pan[1],
            u8::from(self.flipped)
        )
    }
    pub fn decode(text: &str) -> Option<Self> {
        let parts: Vec<_> = text.split_whitespace().collect();
        if parts.len() != 6 || parts[0] != "1" {
            return None;
        }
        let view = Self {
            zoom: parts[1].parse().ok()?,
            angle: parts[2].parse().ok()?,
            pan: [parts[3].parse().ok()?, parts[4].parse().ok()?],
            flipped: match parts[5] {
                "0" => false,
                "1" => true,
                _ => return None,
            },
        };
        if !(0.01..=64.0).contains(&view.zoom)
            || !view.angle.is_finite()
            || view.pan.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return None;
        }
        Some(Self {
            angle: view.angle.rem_euclid(360.0),
            ..view
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: [f64; 2], b: [f64; 2]) {
        assert!(
            (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
            "{a:?} != {b:?}"
        );
    }
    #[test]
    fn transforms_and_inverse_match_across_zoom_rotation_mirror_and_dpi() {
        for zoom in [0.01, 1.0, 4.0, 64.0] {
            for angle in [0.0, 90.0, 37.0] {
                for flipped in [false, true] {
                    for dpi in [1.0, 1.5, 2.0] {
                        let view = View {
                            zoom,
                            angle,
                            flipped,
                            pan: [130.0, -75.0],
                        };
                        let size = [1200.0, 900.0];
                        let center = [550.0, 400.0];
                        for point in [[0.0, 0.0], [1200.0, 900.0], [127.0, 311.0]] {
                            close(
                                view.screen_to_document(
                                    view.document_to_screen(point, size, center, dpi),
                                    size,
                                    center,
                                    dpi,
                                ),
                                point,
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn zoom_keeps_cursor_point_and_clamps_without_accepting_nan() {
        let mut view = View {
            angle: 37.0,
            flipped: true,
            ..Default::default()
        };
        let anchor = [100.0, 140.0];
        let size = [300.0, 200.0];
        let center = [500.0, 400.0];
        let before = view.screen_to_document(anchor, size, center, 2.0);
        view.zoom_about(1.5, anchor, size, center, 2.0);
        close(view.screen_to_document(anchor, size, center, 2.0), before);
        view.zoom_about(1e6, anchor, size, center, 2.0);
        assert_eq!(view.zoom, 64.0);
        view.zoom_about(1e-9, anchor, size, center, 2.0);
        assert_eq!(view.zoom, 0.01);
        let before = view;
        view.zoom_about(f64::NAN, anchor, size, center, 2.0);
        view.pan_by([f64::INFINITY, 0.0]);
        assert_eq!(view, before);
        assert_eq!(View::decode(&view.encode()), Some(view));
        assert!(View::decode("1 NaN 0 0 0 0").is_none());
    }
}
