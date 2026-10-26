use crate::{
    comics::PageInfo,
    document::Document,
    scene::{Point, Rect},
};
impl PageInfo {
    pub fn is_right(&self) -> bool {
        self.first_right == (self.number % 2 == 1)
    }
    pub fn facing(&self) -> Option<u32> {
        let n = if self.is_right() != self.right_binding {
            self.number.checked_sub(1)?
        } else {
            self.number.checked_add(1)?
        };
        (n > 0 && n <= self.total).then_some(n)
    }
    pub fn guides(&self, d: &Document) -> Result<Vec<Rect>, String> {
        if !self.valid() {
            return Err("Invalid page information".into());
        }
        let mm = d.dpi / 25.4;
        let trim = [
            if self.trim_mm[0] == 0. {
                d.width as f64
            } else {
                self.trim_mm[0] * mm
            },
            if self.trim_mm[1] == 0. {
                d.height as f64
            } else {
                self.trim_mm[1] * mm
            },
        ];
        if trim[0] + 2. * self.bleed_mm * mm > d.width as f64 + 1e-6
            || trim[1] + 2. * self.bleed_mm * mm > d.height as f64 + 1e-6
            || 2. * self.safe_mm * mm >= trim[0].min(trim[1])
        {
            return Err("Trim, bleed and safe area must fit the page".into());
        }
        let center = [d.width as f64 / 2., d.height as f64 / 2.];
        let inset = |amount: f64| Rect {
            min: [
                center[0] - trim[0] / 2. + amount,
                center[1] - trim[1] / 2. + amount,
            ],
            max: [
                center[0] + trim[0] / 2. - amount,
                center[1] + trim[1] / 2. - amount,
            ],
        };
        Ok(vec![
            inset(0.),
            inset(-self.bleed_mm * mm),
            inset(self.safe_mm * mm),
        ])
    }
    pub(crate) fn output_pixel(&self, d: &Document, p: Point) -> [u8; 4] {
        if self.include_guides
            && let Ok(guides) = self.guides(d)
            && guides.iter().any(|r| {
                p[0] >= r.min[0] - 0.5
                    && p[0] <= r.max[0] + 0.5
                    && p[1] >= r.min[1] - 0.5
                    && p[1] <= r.max[1] + 0.5
                    && ((p[0] - r.min[0]).abs() <= 0.5 + 1e-9
                        || (p[0] - r.max[0]).abs() <= 0.5 + 1e-9
                        || (p[1] - r.min[1]).abs() <= 0.5 + 1e-9
                        || (p[1] - r.max[1]).abs() <= 0.5 + 1e-9)
            })
        {
            return [0, 100, 200, 255];
        }
        if self.include_number {
            let text = self.number.to_string();
            let origin = [
                d.width as f64 / 2. - text.len() as f64 * 3.,
                d.height as f64 - 9.,
            ];
            let q = [
                (p[0] - origin[0]).floor() as i64,
                (p[1] - origin[1]).floor() as i64,
            ];
            if q[0] >= 0 && q[1] >= 0 && q[1] < 7 {
                let digit = (q[0] / 6) as usize;
                if let Some(b) = text.as_bytes().get(digit) {
                    let masks: [u8; 10] = [
                        0b1111110, 0b0110000, 0b1101101, 0b1111001, 0b0110011, 0b1011011,
                        0b1011111, 0b1110000, 0b1111111, 0b1111011,
                    ];
                    let x = q[0] % 6;
                    let y = q[1];
                    let seg = if y == 0 && x < 5 {
                        Some(6)
                    } else if x == 4 && y <= 3 {
                        Some(5)
                    } else if x == 4 && y >= 3 {
                        Some(4)
                    } else if y == 6 && x < 5 {
                        Some(3)
                    } else if x == 0 && y >= 3 {
                        Some(2)
                    } else if x == 0 && y <= 3 {
                        Some(1)
                    } else if y == 3 && x < 5 {
                        Some(0)
                    } else {
                        None
                    };
                    if seg.is_some_and(|n| masks[(b - b'0') as usize] & (1 << n) != 0) {
                        return [0, 0, 0, 255];
                    }
                }
            }
        }
        [0; 4]
    }
}
