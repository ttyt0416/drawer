//! Physical display coordinates, RGB arithmetic averaging; no file/network I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub min: [i32; 2],
    pub max: [i32; 2],
}
impl Bounds {
    pub fn valid(self) -> bool {
        (0..2).all(|n| {
            self.max[n] as i64 - self.min[n] as i64 > 0
                && self.max[n] as i64 - self.min[n] as i64 <= 131072
        })
    }
    pub fn contains(self, p: [i32; 2]) -> bool {
        (0..2).all(|n| p[n] >= self.min[n] && p[n] < self.max[n])
    }
}
pub fn mean(
    center: [i32; 2],
    radius: u8,
    bounds: Bounds,
    mut read: impl FnMut([i32; 2]) -> Result<Option<[u8; 3]>, String>,
) -> Result<[u8; 3], String> {
    if radius > 4 || !bounds.valid() || !bounds.contains(center) {
        return Err("Screen point or kernel is unavailable".into());
    }
    let mut sum = [0u32; 3];
    let mut count = 0;
    for y in (center[1] as i64 - radius as i64).max(bounds.min[1] as i64)
        ..=(center[1] as i64 + radius as i64).min(bounds.max[1] as i64 - 1)
    {
        for x in (center[0] as i64 - radius as i64).max(bounds.min[0] as i64)
            ..=(center[0] as i64 + radius as i64).min(bounds.max[0] as i64 - 1)
        {
            if let Some(c) = read([x as i32, y as i32])? {
                for n in 0..3 {
                    sum[n] += c[n] as u32;
                }
                count += 1;
            }
        }
    }
    if count == 0 {
        return Err("No readable display pixels".into());
    }
    Ok(sum.map(|v| ((v + count / 2) / count) as u8))
}
pub fn verify() -> Result<(), String> {
    let bounds = Bounds {
        min: [-3, 0],
        max: [3, 3],
    };
    let color = mean([0, 1], 1, bounds, |p| {
        Ok(Some(if p[0] == 0 { [255, 0, 0] } else { [0, 0, 255] }))
    })?;
    if color != [85, 0, 170] {
        return Err("Screen RGB arithmetic average mismatch".into());
    }
    if mean([-3, 0], 1, bounds, |p| {
        Ok(Some([(p[0] + 3) as u8 * 100, p[1] as u8 * 100, 0]))
    })? != [50, 50, 0]
    {
        return Err("Screen edge kernel mismatch".into());
    }
    if mean([0, 1], 1, bounds, |p| {
        Ok((p[0] != 0).then_some([10, 20, 30]))
    })? != [10, 20, 30]
    {
        return Err("Monitor-gap exclusion mismatch".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negative_desktop_edges_gaps_and_display_rgb_mean() {
        verify().unwrap();
        let b = Bounds {
            min: [-1920, -1080],
            max: [3840, 2160],
        };
        for p in [[-1919, -1079], [0, 0], [3839, 2159]] {
            assert_eq!(
                mean(p, 4, b, |_| Ok(Some([21, 83, 177]))).unwrap(),
                [21, 83, 177]
            );
        }
        assert!(mean([0, 0], 1, b, |_| Err("unavailable".into())).is_err());
        assert!(mean([0, 0], 1, b, |_| Ok(None)).is_err());
        assert!(mean([0, 0], 5, b, |_| unreachable!()).is_err());
    }
}
