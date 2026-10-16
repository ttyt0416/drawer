//! Canonical unassociated 8-bit RGBA with independent RGB/HSV/HSL picker metadata.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Model {
    #[default]
    Rgb,
    Hsv,
    Hsl,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub preserve_lightness: bool,
    pub model: Model,
    pub hue: f64,
}
impl Settings {
    pub fn valid(self) -> bool {
        self.hue.is_finite() && (0.0..=360.0).contains(&self.hue)
    }
    pub fn observe(&mut self, color: [u8; 4]) {
        let max = *color[..3].iter().max().unwrap();
        let min = *color[..3].iter().min().unwrap();
        if max != min {
            self.hue = hsv(color, self.hue)[0]
        }
    }
}
pub fn rgb(values: [f64; 4]) -> Result<[u8; 4], String> {
    if values
        .iter()
        .any(|n| !n.is_finite() || !(0.0..=255.0).contains(n) || n.fract() != 0.0)
    {
        return Err("RGBA requires four whole numbers from 0 to 255".into());
    }
    Ok(values.map(|v| v as u8))
}
pub fn hsv(color: [u8; 4], last_hue: f64) -> [f64; 3] {
    let r = color[0] as f64 / 255.0;
    let g = color[1] as f64 / 255.0;
    let b = color[2] as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        last_hue
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    [h, if max == 0.0 { 0.0 } else { d / max }, max]
}
pub fn from_hsv([h, s, v]: [f64; 3], alpha: u8) -> Result<[u8; 4], String> {
    if !h.is_finite()
        || !(0.0..=360.0).contains(&h)
        || !(0.0..=1.0).contains(&s)
        || !(0.0..=1.0).contains(&v)
    {
        return Err("HSV requires hue 0–360°, saturation/value 0–100%".into());
    }
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let rgb = match h as u32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    let m = v - c;
    Ok([
        (255.0 * (rgb[0] + m)).round() as u8,
        (255.0 * (rgb[1] + m)).round() as u8,
        (255.0 * (rgb[2] + m)).round() as u8,
        alpha,
    ])
}

pub fn hsl(color: [u8; 4], last_hue: f64) -> [f64; 3] {
    let max = *color[..3].iter().max().unwrap() as f64 / 255.0;
    let min = *color[..3].iter().min().unwrap() as f64 / 255.0;
    let d = max - min;
    let l = (max + min) / 2.0;
    let saturation = if d == 0.0 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs())
    };
    [hsv(color, last_hue)[0], saturation.clamp(0.0, 1.0), l]
}
pub fn from_hsl([h, s, l]: [f64; 3], alpha: u8) -> Result<[u8; 4], String> {
    if !h.is_finite()
        || !(0.0..=360.0).contains(&h)
        || !(0.0..=1.0).contains(&s)
        || !(0.0..=1.0).contains(&l)
    {
        return Err("HSL requires hue 0–360°, saturation/lightness 0–1".into());
    }
    let h = h.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let rgb = match h as u32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    let m = l - c / 2.0;
    Ok([
        (255.0 * (rgb[0] + m)).round() as u8,
        (255.0 * (rgb[1] + m)).round() as u8,
        (255.0 * (rgb[2] + m)).round() as u8,
        alpha,
    ])
}
#[derive(Clone, Copy, Debug)]
pub struct Lightness {
    pub hue: f64,
    pub saturation: f64,
    pub alpha: u8,
}
impl Lightness {
    pub fn begin(color: [u8; 4], hue: f64) -> Result<Self, String> {
        if !hue.is_finite() || !(0.0..=360.0).contains(&hue) {
            return Err("Invalid lightness hue".into());
        }
        let [h, s, _] = hsl(color, hue);
        Ok(Self {
            hue: h,
            saturation: s,
            alpha: color[3],
        })
    }
    pub fn at(self, lightness: f64) -> Result<[u8; 4], String> {
        from_hsl([self.hue, self.saturation, lightness], self.alpha)
    }
}
pub fn verify_hsl() -> Result<(), String> {
    for r in (0..=255).step_by(17) {
        for g in (0..=255).step_by(17) {
            for b in (0..=255).step_by(17) {
                for alpha in [0, 128, 255] {
                    let c = [r, g, b, alpha];
                    if from_hsl(hsl(c, 217.0), alpha)? != c {
                        return Err("HSL round trip changed RGBA".into());
                    }
                }
            }
        }
    }
    let light = Lightness::begin([220, 30, 40, 128], 217.0)?;
    for l in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0, 0.5] {
        let c = light.at(l)?;
        if c[3] != 128 {
            return Err("Lightness changed alpha".into());
        }
        if l > 0.0 && l < 1.0 {
            let [h, s, _] = hsl(c, light.hue);
            if (h - light.hue).abs() > 1.5 || (s - light.saturation).abs() > 0.025 {
                return Err(
                    "Lightness changed the source hue/saturation beyond 8-bit rounding".into(),
                );
            }
        }
    }
    if from_hsl([0.0, 1.0, 0.5], 0)? != [255, 0, 0, 0]
        || from_hsl([360.0, 1.0, 0.5], 128)? != [255, 0, 0, 128]
    {
        return Err("HSL zero/full-turn mismatch".into());
    }
    Ok(())
}
pub fn hex(color: [u8; 4]) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color[0], color[1], color[2], color[3]
    )
}
pub fn parse_hex(text: &str, current_alpha: u8) -> Result<[u8; 4], String> {
    let s = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if ![6, 8].contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Use #RRGGBB or #RRGGBBAA".into());
    }
    let mut c = [0, 0, 0, current_alpha];
    for (i, v) in c.iter_mut().enumerate().take(s.len() / 2) {
        *v = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "Invalid HEX channel")?;
    }
    Ok(c)
}
/// Unit wheel: hue clockwise from the right, saturation as radius, value unchanged.
pub fn wheel(point: [f64; 2], value: f64, alpha: u8) -> Result<([u8; 4], f64), String> {
    if point.iter().any(|v| !v.is_finite()) {
        return Err("Invalid wheel point".into());
    }
    let hue = point[1].atan2(point[0]).to_degrees().rem_euclid(360.0);
    let c = from_hsv([hue, point[0].hypot(point[1]).min(1.0), value], alpha)?;
    Ok((c, hue))
}
pub fn verify() -> Result<(), String> {
    for r in (0..=255).step_by(17) {
        for g in (0..=255).step_by(17) {
            for b in (0..=255).step_by(17) {
                for a in [0, 128, 255] {
                    let c = [r, g, b, a];
                    if from_hsv(hsv(c, 270.0), a)? != c || parse_hex(&hex(c), 77)? != c {
                        return Err("RGB/HSV/HEX round trip changed canonical RGBA".into());
                    }
                }
            }
        }
    }
    for text in [
        "#00000",
        "#GG0000",
        "#１２３４５６",
        "#00112233ff",
        "#001122zz",
    ] {
        if parse_hex(text, 0).is_ok() {
            return Err("Invalid HEX accepted".into());
        }
    }
    for v in [f64::NAN, f64::INFINITY, -1.0, 256.0, 0.5] {
        if rgb([v, 0.0, 0.0, 255.0]).is_ok() {
            return Err("Invalid RGB number accepted".into());
        }
    }
    let (c, h) = wheel([0.75, 0.0], 1.0, 0)?;
    if c != [255, 64, 64, 0] || h != 0.0 || hsv([100, 100, 100, 0], 217.0)[0] != 217.0 {
        return Err("Wheel/gray hue/transparent RGB mismatch".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrips_hidden_rgb_and_invalid_inputs() {
        verify().unwrap();
        assert_eq!(parse_hex("aBcDeF", 19).unwrap(), [171, 205, 239, 19]);
        assert_eq!(from_hsv([360.0, 1.0, 1.0], 128).unwrap(), [255, 0, 0, 128]);
        for v in [f64::NAN, f64::INFINITY, -0.1, 1.01] {
            assert!(from_hsv([0.0, v, 1.0], 255).is_err());
        }
    }
    #[test]
    fn hsv_primary_axes_gray_and_metadata_only() {
        for (h, c) in [
            (0.0, [255, 0, 0, 0]),
            (60.0, [255, 255, 0, 0]),
            (120.0, [0, 255, 0, 0]),
            (180.0, [0, 255, 255, 0]),
            (240.0, [0, 0, 255, 0]),
            (300.0, [255, 0, 255, 0]),
        ] {
            assert_eq!(from_hsv([h, 1.0, 1.0], 0).unwrap(), c);
            assert!((hsv(c, 73.0)[0] - h).abs() < 1e-9)
        }
        let mut s = Settings {
            preserve_lightness: false,
            model: Model::Hsv,
            hue: 217.0,
        };
        s.observe([100, 100, 100, 0]);
        assert_eq!(s.hue, 217.0);
        s.observe([255, 0, 0, 0]);
        assert_eq!(s.hue, 0.0);
        assert!(!Settings { hue: f64::NAN, ..s }.valid());
    }
    #[test]
    fn composite_sampler_includes_editable_vectors_and_page_objects() {
        use crate::eyedropper::{Settings, Source};
        let mut d = crate::line_tools_verify::fixture().unwrap();
        let eye = Settings {
            source: Source::Composite,
            radius: 0,
        };
        assert_eq!(
            eye.sample(&d, None, [8.0, 20.0], [1; 4]).unwrap(),
            Some([0, 0, 0, 255])
        );
        d.scene.lines.layers[0].hidden = true;
        assert_eq!(eye.sample(&d, None, [8.0, 20.0], [1; 4]).unwrap(), None);
        d.scene.objects.push(crate::scene::PageObject::Line {
            start: [4.0, 20.5],
            end: [60.0, 20.5],
            width_px: 2.0,
            color: [20, 80, 160, 128],
        });
        assert_eq!(
            eye.sample(&d, None, [12.0, 20.0], [1; 4]).unwrap(),
            Some([20, 80, 160, 128])
        );
    }
}

#[cfg(test)]
mod hsl_tests {
    use super::*;
    #[test]
    fn hsl_roundtrip_lightness_endpoints_and_rejected_values() {
        verify_hsl().unwrap();
        for c in [
            [0, 0, 0, 0],
            [255; 4],
            [128, 128, 128, 128],
            [255, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 0],
        ] {
            assert_eq!(from_hsl(hsl(c, 217.0), c[3]).unwrap(), c);
            for v in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
                assert!(from_hsl([217.0, v, 0.5], c[3]).is_err());
                assert!(from_hsl([217.0, 0.5, v], c[3]).is_err());
            }
            let t = Lightness::begin(c, 217.0).unwrap();
            assert_eq!(t.at(0.0).unwrap(), [0, 0, 0, c[3]]);
            assert_eq!(t.at(1.0).unwrap(), [255, 255, 255, c[3]]);
            assert_eq!(t.at(hsl(c, 217.0)[2]).unwrap(), c);
        }
        assert_eq!(hsl([128, 128, 128, 0], 217.0), [217.0, 0.0, 128.0 / 255.0]);
        for h in [f64::NAN, -1.0, 361.0] {
            assert!(from_hsl([h, 1.0, 0.5], 0).is_err());
        }
    }
}
