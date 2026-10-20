//! Independently expressed sRGB/Oklab mathematics from the public Oklab definition.
//! https://bottosson.github.io/posts/oklab/ (public-domain reference matrices).
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Space {
    #[default]
    Standard,
    Perceptual,
    LabD65,
}
impl Space {
    pub const ALL: [Self; 3] = [Self::Standard, Self::LabD65, Self::Perceptual];
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Perceptual => "Perceptual Oklab",
            Self::LabD65 => "Perceptual CIELAB D65",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    #[default]
    Normal,
    High,
}
impl Quality {
    pub const ALL: [Self; 2] = [Self::Normal, Self::High];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::High => "High quality",
        }
    }
}
fn matrix(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
}
pub(crate) fn linear(x: f64) -> f64 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
pub(crate) fn encoded(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}
pub(crate) fn coordinates(space: Space, rgb: [u8; 3]) -> [f64; 3] {
    coordinates_float(space, rgb.map(|n| n as f64 / 255.))
}
pub(crate) fn coordinates_float(space: Space, c: [f64; 3]) -> [f64; 3] {
    match space {
        Space::LabD65 => crate::lab_mix::coordinates_float(c),
        Space::Standard => c,
        Space::Perceptual => {
            let lms = matrix(
                [
                    [0.4122214708, 0.5363325363, 0.0514459929],
                    [0.2119034982, 0.6806995451, 0.1073969566],
                    [0.0883024619, 0.2817188376, 0.6299787005],
                ],
                c.map(linear),
            )
            .map(f64::cbrt);
            matrix(
                [
                    [0.2104542553, 0.7936177850, -0.0040720468],
                    [1.9779984951, -2.4285922050, 0.4505937099],
                    [0.0259040371, 0.7827717662, -0.8086757660],
                ],
                lms,
            )
        }
    }
}
pub(crate) fn mix_float(
    space: Space,
    a: crate::color_precision::Sample,
    b: crate::color_precision::Sample,
    weight: f64,
) -> Result<crate::color_precision::Sample, String> {
    if !weight.is_finite() {
        return Err("Nonfinite color mixing weight".into());
    }
    if weight <= 0. {
        return Ok(a);
    }
    if weight >= 1. {
        return Ok(b);
    }
    let aa = a.alpha() * (1. - weight);
    let ba = b.alpha() * weight;
    let alpha = aa + ba;
    if alpha == 0. {
        return Ok(Default::default());
    }
    let a = coordinates_float(space, a.rgb());
    let b = coordinates_float(space, b.rgb());
    let rgb = rgb_float(space, [0, 1, 2].map(|i| (a[i] * aa + b[i] * ba) / alpha));
    crate::color_precision::Sample::new([rgb[0], rgb[1], rgb[2], alpha])
}
fn rgb(space: Space, c: [f64; 3]) -> [u8; 3] {
    rgb_float(space, c).map(|v| (v * 255.0).round() as u8)
}
pub(crate) fn rgb_float(space: Space, c: [f64; 3]) -> [f64; 3] {
    let c = match space {
        Space::LabD65 => return crate::lab_mix::rgb_float(c),
        Space::Standard => c,
        Space::Perceptual => {
            let lms = matrix(
                [
                    [1.0, 0.3963377774, 0.2158037573],
                    [1.0, -0.1055613458, -0.0638541728],
                    [1.0, -0.0894841775, -1.2914855480],
                ],
                c,
            )
            .map(|x| x * x * x);
            matrix(
                [
                    [4.0767416621, -3.3077115913, 0.2309699292],
                    [-1.2684380046, 2.6097574011, -0.3413193965],
                    [-0.0041960863, -0.7034186147, 1.7076147010],
                ],
                lms,
            )
            .map(encoded)
        }
    };
    c.map(|x| x.clamp(0.0, 1.0))
}
pub fn mix(space: Space, a: [u8; 4], b: [u8; 4], weight: f64) -> [u8; 4] {
    if !weight.is_finite() || weight <= 0.0 {
        return a;
    }
    if weight >= 1.0 {
        return b;
    }
    let aa = a[3] as f64 / 255.0 * (1.0 - weight);
    let ba = b[3] as f64 / 255.0 * weight;
    let alpha = aa + ba;
    if alpha <= 0.0 {
        return [0; 4];
    }
    let ac = coordinates(space, [a[0], a[1], a[2]]);
    let bc = coordinates(space, [b[0], b[1], b[2]]);
    let c = rgb(
        space,
        std::array::from_fn(|n| (ac[n] * aa + bc[n] * ba) / alpha),
    );
    [c[0], c[1], c[2], (alpha * 255.0).round() as u8]
}
pub fn average(space: Space, samples: impl IntoIterator<Item = ([u8; 4], f64)>) -> [u8; 4] {
    let mut sum = [0.0; 3];
    let (mut alpha, mut weights) = (0.0, 0.0);
    for (p, w) in samples {
        if !w.is_finite() || w <= 0.0 {
            continue;
        }
        weights += w;
        let a = p[3] as f64 / 255.0 * w;
        alpha += a;
        let c = coordinates(space, [p[0], p[1], p[2]]);
        for n in 0..3 {
            sum[n] += c[n] * a;
        }
    }
    if alpha <= 0.0 || weights <= 0.0 {
        return [0; 4];
    }
    let c = rgb(space, sum.map(|v| v / alpha));
    [c[0], c[1], c[2], (alpha / weights * 255.0).round() as u8]
}

pub fn verify() -> Result<(), String> {
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    if mix(Space::Standard, red, blue, 0.5) != [128, 0, 128, 255]
        || mix(Space::Perceptual, red, blue, 0.5) == [128, 0, 128, 255]
    {
        return Err("Mix spaces did not differ".into());
    }
    let c = coordinates(Space::Perceptual, [255, 0, 0]);
    if (c[0] - 0.62795536).abs() > 1e-7
        || (c[1] - 0.22486306).abs() > 1e-7
        || (c[2] - 0.12584630).abs() > 1e-7
    {
        return Err("Oklab primary reference mismatch".into());
    }
    for space in Space::ALL {
        for rgb0 in [
            [0, 0, 0],
            [255, 255, 255],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [40, 120, 210],
        ] {
            if rgb(space, coordinates(space, rgb0)) != rgb0 {
                return Err("Mix coordinates failed round trip".into());
            }
        }
        if mix(space, red, [0, 255, 0, 0], 0.5) != [255, 0, 0, 128]
            || average(space, [(red, 1.0), ([0, 255, 0, 0], 1.0)]) != [255, 0, 0, 128]
        {
            return Err("Transparent hidden RGB polluted mix".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spaces_reference_primaries_round_trip_and_premultiplied_alpha() {
        verify().unwrap();
    }
    #[test]
    fn endpoints_invalid_weights_and_empty_averages() {
        for space in Space::ALL {
            let a = [40, 80, 120, 0];
            let b = [200, 100, 40, 128];
            assert_eq!(mix(space, a, b, 0.0), a);
            assert_eq!(mix(space, a, b, 1.0), b);
            assert_eq!(mix(space, a, b, f64::NAN), a);
            assert_eq!(average(space, [(b, 0.0), (a, f64::INFINITY)]), [0; 4]);
        }
    }
}
