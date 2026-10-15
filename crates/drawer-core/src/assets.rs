pub const APP_NAME: &str = "Drawer";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BrushTip {
    Pen,
    Pencil,
    Eraser,
    Wash,
}

impl BrushTip {
    pub const ALL: [Self; 4] = [Self::Pen, Self::Pencil, Self::Eraser, Self::Wash];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pen => "Pen",
            Self::Pencil => "Pencil",
            Self::Eraser => "Eraser",
            Self::Wash => "Wash",
        }
    }
    /// Display names can evolve without changing saved command/preset IDs.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wash => "Airbrush",
            _ => self.name(),
        }
    }
    pub fn mask(self, size: usize) -> Vec<u8> {
        assert!((2..=256).contains(&size));
        let mut pixels = Vec::with_capacity(size * size);
        for y in 0..size {
            for x in 0..size {
                let nx = (x as f32 + 0.5) * 2.0 / size as f32 - 1.0;
                let ny = (y as f32 + 0.5) * 2.0 / size as f32 - 1.0;
                let radius = (nx * nx + ny * ny).sqrt();
                let value = if radius >= 1.0 {
                    0.0
                } else {
                    match self {
                        Self::Pen | Self::Eraser => ((1.0 - radius) * 8.0).clamp(0.0, 1.0),
                        Self::Pencil => {
                            let grain = ((x * 17 + y * 31 + x * y * 7) % 23) as f32 / 22.0;
                            (1.0 - radius).sqrt() * (0.4 + 0.6 * grain)
                        }
                        Self::Wash => (1.0 - radius * radius).powi(3) * 0.65,
                    }
                };
                pixels.push((value * 255.0).round() as u8);
            }
        }
        pixels
    }
}

pub fn application_icon(size: usize) -> Vec<u8> {
    let mut rgba = vec![0; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let nx = x as f32 / size as f32;
            let ny = y as f32 / size as f32;
            let pen = (nx + ny - 1.0).abs() < 0.09 && (0.2..0.8).contains(&nx);
            let color = if pen {
                [236, 247, 242, 255]
            } else {
                [30, 84, 75, 255]
            };
            rgba[(y * size + x) * 4..][..4].copy_from_slice(&color);
        }
    }
    rgba
}
pub fn tool_icon(tip: BrushTip, size: usize) -> Vec<[u8; 4]> {
    assert!((8..=64).contains(&size));
    (0..size * size)
        .map(|n| {
            let x = (n % size) as f64 / size as f64;
            let y = (n / size) as f64 / size as f64;
            let (inside, color) = match tip {
                BrushTip::Pen => (
                    (x + y - 1.0).abs() < 0.15 && (0.2..0.85).contains(&x),
                    [100, 230, 170, 255],
                ),
                BrushTip::Pencil => (
                    (x + y - 1.0).abs() < 0.09 && (0.12..0.88).contains(&x),
                    [245, 205, 90, 255],
                ),
                BrushTip::Eraser => (
                    (0.2..0.8).contains(&x) && (0.3..0.7).contains(&y),
                    [250, 120, 165, 255],
                ),
                BrushTip::Wash => (
                    (x - 0.5).hypot(y - 0.65) < 0.23
                        || ((0.18..0.6).contains(&y) && (x - 0.5).abs() < (y - 0.15) * 0.5),
                    [100, 185, 255, 255],
                ),
            };
            if inside { color } else { [0; 4] }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_tips_are_self_contained_and_distinct() {
        let masks: Vec<_> = BrushTip::ALL.into_iter().map(|tip| tip.mask(32)).collect();
        for mask in &masks {
            assert_eq!(mask.len(), 1024);
            assert_eq!(mask[0], 0);
            assert!(mask.iter().any(|&alpha| alpha > 0));
        }
        assert_ne!(masks[0], masks[1]);
        assert_ne!(masks[0], masks[3]);
        assert_eq!(masks[1], BrushTip::Pencil.mask(32));
        assert_eq!(application_icon(32).len(), 4096);
    }
}
