//! Canonical finite values; UI units and input methods never own a second value.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Pixels,
    Millimeters,
    Inches,
    Percent,
    Scalar,
}
impl Unit {
    pub const SIZE: [Self; 3] = [Self::Pixels, Self::Millimeters, Self::Inches];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pixels => "px",
            Self::Millimeters => "mm",
            Self::Inches => "in",
            Self::Percent => "%",
            Self::Scalar => "×",
        }
    }
    pub fn factor(self, dpi: f64) -> Result<f64, String> {
        match self {
            Self::Pixels | Self::Scalar => Ok(1.0),
            Self::Percent => Ok(0.01),
            _ if !dpi.is_finite() || dpi <= 0.0 => {
                Err("Physical units need positive finite DPI".into())
            }
            Self::Millimeters if dpi / 25.4 == 0.0 => Err("DPI is below numeric precision".into()),
            Self::Millimeters => Ok(dpi / 25.4),
            Self::Inches => Ok(dpi),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}
impl Range {
    pub fn valid(self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.min <= self.max
    }
    pub fn contains(self, value: f64) -> bool {
        self.valid() && value.is_finite() && value >= self.min && value <= self.max
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Reject,
    Clamp,
}
#[derive(Clone, Copy, Debug)]
pub struct Scalar {
    value: f64,
    range: Range,
}
impl Scalar {
    pub fn new(value: f64, range: Range) -> Result<Self, String> {
        if !range.contains(value) {
            return Err(format!(
                "Enter a finite value from {} to {}",
                range.min, range.max
            ));
        }
        Ok(Self { value, range })
    }
    pub fn value(self) -> f64 {
        self.value
    }
    pub fn display(self, unit: Unit, dpi: f64) -> Result<(f64, Range), String> {
        let factor = unit.factor(dpi)?;
        let value = self.value / factor;
        let range = Range {
            min: self.range.min / factor,
            max: self.range.max / factor,
        };
        if !range.valid() || !value.is_finite() {
            return Err("Displayed unit exceeds numeric limits".into());
        }
        Ok((value, range))
    }
    pub fn set(&mut self, value: f64) -> Result<bool, String> {
        if !self.range.contains(value) {
            return Err(format!(
                "Enter a finite value from {} to {}",
                self.range.min, self.range.max
            ));
        }
        if (value - self.value).abs() <= 1e-12 * self.value.abs().max(value.abs()).max(1.0) {
            return Ok(false);
        }
        self.value = value;
        Ok(true)
    }
    pub fn parse(&mut self, text: &str, unit: Unit, dpi: f64) -> Result<bool, String> {
        let value = text
            .trim()
            .parse::<f64>()
            .map_err(|_| "Enter a valid finite number")?;
        self.set_display(value, unit, dpi)
    }
    pub fn set_display(&mut self, value: f64, unit: Unit, dpi: f64) -> Result<bool, String> {
        self.set(value * unit.factor(dpi)?)
    }
    pub fn step(&mut self, delta: f64) -> Result<bool, String> {
        if !delta.is_finite() {
            return Err("Invalid numeric increment".into());
        }
        self.set((self.value + delta).clamp(self.range.min, self.range.max))
    }
    pub fn reconfigure(&mut self, range: Range, policy: Policy) -> Result<bool, String> {
        if !range.valid() {
            return Err("Invalid dependent numeric range".into());
        }
        if policy == Policy::Reject && !range.contains(self.value) {
            return Err(format!(
                "Current value is outside the new range {} to {}",
                range.min, range.max
            ));
        }
        let value = self.value.clamp(range.min, range.max);
        let changed = value != self.value;
        self.value = value;
        self.range = range;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unit_round_trip_invalid_input_and_duplicate_updates() {
        let mut n = Scalar::new(
            300.0,
            Range {
                min: 0.1,
                max: 4096.0,
            },
        )
        .unwrap();
        assert_eq!(n.display(Unit::Inches, 300.0).unwrap().0, 1.0);
        assert_eq!(n.display(Unit::Millimeters, 300.0).unwrap().0, 25.4);
        assert!(!n.parse("25.4", Unit::Millimeters, 300.0).unwrap());
        for bad in ["", "abc", "NaN", "inf", "-inf", "1e999", "0", "4097"] {
            assert!(n.parse(bad, Unit::Pixels, 300.0).is_err(), "{bad}");
            assert_eq!(n.value(), 300.0);
        }
        assert!(n.parse("2", Unit::Inches, 300.0).unwrap());
        assert_eq!(n.value(), 600.0);
        assert!(n.display(Unit::Millimeters, 0.0).is_err());
        assert!(n.display(Unit::Millimeters, f64::NAN).is_err());
        assert!(Unit::Millimeters.factor(f64::from_bits(1)).is_err());
        assert!(Scalar::new(0.0, Range { min: 1.0, max: 0.0 }).is_err());
    }
    #[test]
    fn dependent_bounds_reject_atomically_or_clamp_all_units_and_steps() {
        let mut n = Scalar::new(
            50.0,
            Range {
                min: 0.0,
                max: 100.0,
            },
        )
        .unwrap();
        let narrow = Range {
            min: 0.0,
            max: 10.0,
        };
        assert!(n.reconfigure(narrow, Policy::Reject).is_err());
        assert_eq!(
            n.display(Unit::Percent, 300.0).unwrap(),
            (
                5000.0,
                Range {
                    min: 0.0,
                    max: 10000.0
                }
            )
        );
        assert!(n.reconfigure(narrow, Policy::Clamp).unwrap());
        assert_eq!(
            n.display(Unit::Percent, 300.0).unwrap(),
            (
                1000.0,
                Range {
                    min: 0.0,
                    max: 1000.0
                }
            )
        );
        assert!(!n.step(1.0).unwrap());
        assert!(n.step(-0.5).unwrap());
        let mut typed = Scalar::new(10.0, narrow).unwrap();
        typed.parse("950", Unit::Percent, 300.0).unwrap();
        assert_eq!(typed.value(), n.value());
        assert!(
            n.reconfigure(
                Range {
                    min: f64::NAN,
                    max: 1.0
                },
                Policy::Clamp
            )
            .is_err()
        );
        assert_eq!(n.value(), 9.5);
        assert!(n.step(f64::INFINITY).is_err());
    }
}
