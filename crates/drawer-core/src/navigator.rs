use crate::{scene::Point, view::View};

pub fn visible_polygon(view: View, size: Point, viewport: [Point; 2], ppp: f64) -> [Point; 4] {
    let [min, max] = viewport;
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    [min, [max[0], min[1]], max, [min[0], max[1]]]
        .map(|p| view.screen_to_document(p, size, center, ppp))
}
/// Intersect the visible quadrilateral with the image. The actual quadrilateral
/// remains available separately, even when the entire image fits inside it.
pub fn clipped_polygon(polygon: [Point; 4], size: Point) -> Vec<Point> {
    let mut points = polygon.to_vec();
    for (axis, boundary, keep_greater) in [
        (0, 0.0, true),
        (0, size[0], false),
        (1, 0.0, true),
        (1, size[1], false),
    ] {
        let input = std::mem::take(&mut points);
        if input.is_empty() {
            break;
        }
        let inside = |p: Point| {
            if keep_greater {
                p[axis] >= boundary
            } else {
                p[axis] <= boundary
            }
        };
        let mut previous = input[input.len() - 1];
        for next in input {
            if inside(previous) != inside(next) {
                let fraction = (boundary - previous[axis]) / (next[axis] - previous[axis]);
                let mut point = [
                    previous[0] + fraction * (next[0] - previous[0]),
                    previous[1] + fraction * (next[1] - previous[1]),
                ];
                point[axis] = boundary;
                points.push(point);
            }
            if inside(next) {
                points.push(next);
            }
            previous = next;
        }
    }
    points
}
impl View {
    pub fn center_on(&mut self, point: Point, size: Point, center: Point, ppp: f64) {
        if point.iter().any(|v| !v.is_finite()) {
            return;
        }
        self.pan = [0.0; 2];
        let projected = self.document_to_screen(point, size, center, ppp);
        self.pan_by([center[0] - projected[0], center[1] - projected[1]]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_outline_and_centering_match_at_extreme_zoom_rotation_mirror_and_display_dpi() {
        let size = [1200.0, 900.0];
        let viewport = [[190.0, 140.0], [950.0, 730.0]];
        let center = [570.0, 435.0];
        for zoom in [0.01, 1.0, 64.0] {
            for angle in [0.0, 37.0, 90.0] {
                for flipped in [false, true] {
                    for ppp in [1.0, 1.5, 2.0] {
                        let mut view = View {
                            zoom,
                            angle,
                            flipped,
                            pan: [170.0, -120.0],
                        };
                        let corners = visible_polygon(view, size, viewport, ppp);
                        for (doc, screen) in corners.into_iter().zip([
                            viewport[0],
                            [950.0, 140.0],
                            viewport[1],
                            [190.0, 730.0],
                        ]) {
                            let result = view.document_to_screen(doc, size, center, ppp);
                            assert!(
                                (result[0] - screen[0]).abs() < 1e-6
                                    && (result[1] - screen[1]).abs() < 1e-6
                            );
                        }
                        let clipped = clipped_polygon(corners, size);
                        assert!(clipped.iter().all(|p| p[0] >= 0.0
                            && p[1] >= 0.0
                            && p[0] <= 1200.0
                            && p[1] <= 900.0));
                        view.center_on([750.0, 600.0], size, center, ppp);
                        let target = view.document_to_screen([750.0, 600.0], size, center, ppp);
                        assert!(
                            (target[0] - center[0]).abs() < 1e-6
                                && (target[1] - center[1]).abs() < 1e-6
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn clipping_handles_full_image_and_no_intersection_without_clamping_a_rotated_outline() {
        let square = [
            [-100.0, -100.0],
            [200.0, -100.0],
            [200.0, 200.0],
            [-100.0, 200.0],
        ];
        assert_eq!(clipped_polygon(square, [100.0, 100.0]).len(), 4);
        let outside = [
            [200.0, 200.0],
            [250.0, 200.0],
            [250.0, 250.0],
            [200.0, 250.0],
        ];
        assert!(clipped_polygon(outside, [100.0, 100.0]).is_empty());
        let diamond = [[50.0, -25.0], [125.0, 50.0], [50.0, 125.0], [-25.0, 50.0]];
        assert_eq!(clipped_polygon(diamond, [100.0, 100.0]).len(), 8);
    }
}
