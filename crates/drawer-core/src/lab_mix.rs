//! CIE 1976 Lab with sRGB's D65 white, independently expressed standard equations.
//! CIE: https://cie.co.at/eilv/157 ; RGB matrices: https://www.w3.org/TR/css-color-4/#color-conversion-code
const WHITE: [f64; 3] = [0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290];
fn matrix(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
}
fn decode(n: u8) -> f64 {
    let v = n as f64 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
pub fn coordinates(rgb: [u8; 3]) -> [f64; 3] {
    let xyz = matrix(
        [
            [506752.0 / 1228815.0, 87881.0 / 245763.0, 12673.0 / 70218.0],
            [87098.0 / 409605.0, 175762.0 / 245763.0, 12673.0 / 175545.0],
            [7918.0 / 409605.0, 87881.0 / 737289.0, 1001167.0 / 1053270.0],
        ],
        rgb.map(decode),
    );
    let f = std::array::from_fn::<_, 3, _>(|n| {
        let v = xyz[n] / WHITE[n];
        if v > 216.0 / 24389.0 {
            v.cbrt()
        } else {
            v * 841.0 / 108.0 + 4.0 / 29.0
        }
    });
    [
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        200.0 * (f[1] - f[2]),
    ]
}
pub fn rgb(lab: [f64; 3]) -> [u8; 3] {
    rgb_float(lab).map(|v| (v * 255.0).round() as u8)
}
pub(crate) fn rgb_float(lab: [f64; 3]) -> [f64; 3] {
    let y = (lab[0] + 16.0) / 116.0;
    let f = [y + lab[1] / 500.0, y, y - lab[2] / 200.0];
    let xyz = std::array::from_fn::<_, 3, _>(|n| {
        let v = f[n];
        (if v > 6.0 / 29.0 {
            v.powi(3)
        } else {
            (v - 4.0 / 29.0) * 108.0 / 841.0
        }) * WHITE[n]
    });
    matrix(
        [
            [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
            [
                -851781.0 / 878810.0,
                1648619.0 / 878810.0,
                36519.0 / 878810.0,
            ],
            [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
        ],
        xyz,
    )
    .map(|v| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    })
}
/// Move the post-mix lightness toward the alpha-weighted input L*, retaining a*/b*.
pub fn correct(mut result: [u8; 4], a: [u8; 4], b: [u8; 4], weight: f64, strength: f64) -> [u8; 4] {
    if !weight.is_finite()
        || !strength.is_finite()
        || strength <= 0.0
        || weight <= 0.0
        || weight >= 1.0
        || result[3] == 0
    {
        return result;
    }
    let aa = (1.0 - weight) * a[3] as f64;
    let ba = weight * b[3] as f64;
    let total = aa + ba;
    if total <= 0.0 {
        return result;
    }
    let c = |p: [u8; 4]| coordinates([p[0], p[1], p[2]]);
    let target = (c(a)[0] * aa + c(b)[0] * ba) / total;
    let mut value = c(result);
    value[0] += (target - value[0]) * strength.clamp(0.0, 1.0);
    result[..3].copy_from_slice(&rgb(value));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cie_reference_round_trip_mix_and_alpha() {
        let red = coordinates([255, 0, 0]);
        for (a, b) in red.into_iter().zip([53.24, 80.09, 67.20]) {
            assert!((a - b).abs() < 0.03);
        }
        assert!(coordinates([0; 3]).iter().all(|v| v.abs() < 1e-10));
        let white = coordinates([255; 3]);
        assert!((white[0] - 100.0).abs() < 1e-10);
        assert!(white[1].abs() < 1e-10 && white[2].abs() < 1e-10);
        for r in [0, 64, 128, 255] {
            for g in [0, 64, 128, 255] {
                for b in [0, 64, 128, 255] {
                    assert_eq!(rgb(coordinates([r, g, b])), [r, g, b]);
                }
            }
        }
        use crate::color_mix::{Space, mix};
        assert_eq!(
            mix(Space::LabD65, [0, 0, 0, 255], [255; 4], 0.5),
            [119, 119, 119, 255]
        );
        assert_eq!(
            mix(Space::LabD65, [255, 0, 0, 255], [0, 255, 0, 0], 0.5),
            [255, 0, 0, 128]
        );
        assert_eq!(
            mix(Space::LabD65, [255, 0, 0, 0], [0, 0, 255, 0], 0.5),
            [0; 4]
        );
    }
    #[test]
    fn lightness_strength_endpoints_and_transparent_inputs() {
        let a = [0, 0, 0, 255];
        let b = [255; 4];
        let mixed = [128, 128, 128, 255];
        assert_eq!(correct(mixed, a, b, 0.5, 0.0), mixed);
        assert_eq!(correct(mixed, a, b, 0.5, 1.0), [119, 119, 119, 255]);
        let half = correct(mixed, a, b, 0.5, 0.5);
        assert!(half[0] > 119 && half[0] < 128);
        assert_eq!(half[3], 255);
        for weight in [0.0, 1.0, f64::NAN] {
            assert_eq!(correct(mixed, a, b, weight, 1.0), mixed);
        }
        assert_eq!(correct([120, 40, 90, 0], a, b, 0.5, 1.0), [120, 40, 90, 0]);
        let transparent = [90, 200, 70, 0];
        let result = crate::color_mix::mix(
            crate::color_mix::Space::LabD65,
            transparent,
            [0, 0, 255, 255],
            0.5,
        );
        assert_eq!(
            correct(result, transparent, [0, 0, 255, 255], 0.5, 1.0),
            [0, 0, 255, 128]
        );
    }
}
