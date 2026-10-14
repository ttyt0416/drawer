use crate::{aids::Unit, document::MAX_DIMENSION, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone, Debug)]
pub struct Dimensions {
    pub width: String,
    pub height: String,
    pub dpi: String,
    pub unit: Unit,
    pub locked: bool,
    ratio: f64,
}
fn value(text: &str) -> Result<f64, String> {
    let n = text
        .trim()
        .parse::<f64>()
        .map_err(|_| "Enter a finite number.".to_string())?;
    if !n.is_finite() {
        return Err("Enter a finite number.".into());
    }
    Ok(n)
}
pub fn dpi(text: &str) -> Result<f64, String> {
    let n = value(text)?;
    if n <= 0.0 {
        return Err("DPI must be positive.".into());
    }
    Ok(n)
}
fn rounded_pixels(number: f64) -> Result<u32, String> {
    if !number.is_finite() || !(1.0..=MAX_DIMENSION as f64).contains(&number.round()) {
        return Err("Dimensions must round to 1–32768 pixels.".into());
    }
    Ok(number.round() as u32)
}
fn text(number: f64) -> String {
    number.to_string()
}
impl Dimensions {
    pub fn new(width: u32, height: u32, dpi: f64) -> Self {
        Self {
            width: width.to_string(),
            height: height.to_string(),
            dpi: dpi.to_string(),
            unit: Unit::Pixels,
            locked: false,
            ratio: width as f64 / height as f64,
        }
    }
    pub fn pixels(&self) -> Result<[u32; 2], String> {
        let dpi = dpi(&self.dpi)?;
        let dimension = |s: &str| {
            let n = value(s)?;
            if n <= 0.0 || (self.unit == Unit::Pixels && n.fract() != 0.0) {
                return Err("Use positive dimensions; pixels must be whole numbers.".into());
            }
            rounded_pixels(self.unit.to_pixels(n, dpi))
        };
        Ok([dimension(&self.width)?, dimension(&self.height)?])
    }
    pub fn set_pixels(&mut self, pixels: [u32; 2], dpi: f64) {
        self.dpi = dpi.to_string();
        self.width = text(self.unit.from_pixels(pixels[0] as f64, dpi));
        self.height = text(self.unit.from_pixels(pixels[1] as f64, dpi));
        self.ratio = pixels[0] as f64 / pixels[1] as f64;
    }
    pub fn switch_unit(&mut self, unit: Unit) -> Result<(), String> {
        let pixels = self.pixels()?;
        let dpi = dpi(&self.dpi)?;
        self.unit = unit;
        self.set_pixels(pixels, dpi);
        Ok(())
    }
    pub fn lock(&mut self, locked: bool) -> Result<(), String> {
        if locked {
            let [w, h] = self.pixels()?;
            self.ratio = w as f64 / h as f64;
        }
        self.locked = locked;
        Ok(())
    }
    pub fn edited_axis(&mut self, axis: usize) -> Result<(), String> {
        if !self.locked {
            return Ok(());
        }
        let dpi = dpi(&self.dpi)?;
        let n = value(if axis == 0 { &self.width } else { &self.height })?;
        let other = if axis == 0 {
            n / self.ratio
        } else {
            n * self.ratio
        };
        let pixels = rounded_pixels(self.unit.to_pixels(other, dpi))?;
        let formatted = text(self.unit.from_pixels(pixels as f64, dpi));
        if axis == 0 {
            self.height = formatted;
        } else {
            self.width = formatted;
        }
        Ok(())
    }
    /// The edited DPI is already in self.dpi; previous is the value before the edit.
    pub fn edited_dpi(&mut self, previous: &str, keep_pixels: bool) -> Result<(), String> {
        let old = dpi(previous)?;
        let new = dpi(&self.dpi)?;
        let mut before = self.clone();
        before.dpi = previous.to_string();
        let pixels = before.pixels()?;
        let target = if keep_pixels {
            pixels
        } else {
            [
                rounded_pixels(pixels[0] as f64 * new / old)?,
                rounded_pixels(pixels[1] as f64 * new / old)?,
            ]
        };
        self.set_pixels(target, new);
        Ok(())
    }
    pub fn edge_pixels(&self, text: &str) -> Result<i64, String> {
        let n = value(text)?;
        if self.unit == Unit::Pixels && n.fract() != 0.0 {
            return Err("Pixel increments must be whole numbers.".into());
        }
        let n = self.unit.to_pixels(n, dpi(&self.dpi)?).round();
        if !n.is_finite() || n.abs() > MAX_DIMENSION as f64 * 2.0 {
            return Err("Edge increment exceeds the supported size.".into());
        }
        Ok(n as i64)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    pub unit: Unit,
}
impl Preset {
    pub fn from_dimensions(name: String, size: &Dimensions) -> Result<Self, String> {
        let [width, height] = size.pixels()?;
        let p = Self {
            name,
            width,
            height,
            dpi: dpi(&size.dpi)?,
            unit: size.unit,
        };
        if !p.valid() {
            return Err("Use a preset name from 1 to 128 characters.".into());
        }
        Ok(p)
    }
    fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.len() <= 128
            && !self.name.contains('\0')
            && (1..=MAX_DIMENSION).contains(&self.width)
            && (1..=MAX_DIMENSION).contains(&self.height)
            && self.dpi.is_finite()
            && self.dpi > 0.0
    }
    pub fn apply(&self, size: &mut Dimensions) {
        size.unit = self.unit;
        size.set_pixels([self.width, self.height], self.dpi);
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetBook {
    pub version: u32,
    pub entries: Vec<Preset>,
}
impl Default for PresetBook {
    fn default() -> Self {
        Self {
            version: 1,
            entries: vec![
                Preset {
                    name: "1200 × 900".into(),
                    width: 1200,
                    height: 900,
                    dpi: 300.0,
                    unit: Unit::Pixels,
                },
                Preset {
                    name: "A4 · 300 DPI".into(),
                    width: 2480,
                    height: 3508,
                    dpi: 300.0,
                    unit: Unit::Millimeters,
                },
                Preset {
                    name: "Square · 2048".into(),
                    width: 2048,
                    height: 2048,
                    dpi: 300.0,
                    unit: Unit::Pixels,
                },
            ],
        }
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("json.write.tmp");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = result.and_then(|_| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
impl PresetBook {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.entries.len() <= 64
            && self.entries.iter().all(Preset::valid)
            && self
                .entries
                .iter()
                .map(|p| p.name.as_str())
                .collect::<HashSet<_>>()
                .len()
                == self.entries.len()
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Canvas presets are too large"));
        }
        let book: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if !book.valid() {
            return Err(io::Error::other("Invalid canvas preset file"));
        }
        Ok(book)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match Self::read(&storage.root().join("settings/canvas-presets.json")) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn export(&self, path: &Path) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid canvas presets"));
        }
        atomic(path, &serde_json::to_vec(self).map_err(io::Error::other)?)
    }
    pub fn save(&self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid canvas presets"));
        }
        let path = storage.root().join("settings/canvas-presets.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(io::Error::other(
                    "Preset settings exceed the recovery limit; original preserved",
                ));
            }
            let name = if Self::read(&path).is_ok() {
                "canvas-presets.previous.json"
            } else {
                "canvas-presets.damaged.json"
            };
            atomic(
                &storage.root().join("settings").join(name),
                &fs::read(&path)?,
            )?;
        }
        self.export(&path)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    #[test]
    fn physical_units_ratio_and_dpi_policies_round_consistently() {
        let mut size = Dimensions::new(300, 150, 300.0);
        size.switch_unit(Unit::Inches).unwrap();
        assert_eq!(size.pixels().unwrap(), [300, 150]);
        size.lock(true).unwrap();
        size.width = "2".into();
        size.edited_axis(0).unwrap();
        assert_eq!(size.pixels().unwrap(), [600, 300]);
        size.dpi = "600".into();
        size.edited_dpi("300", true).unwrap();
        assert_eq!(size.pixels().unwrap(), [600, 300]);
        assert_eq!(size.width, "1");
        size.dpi = "300".into();
        size.edited_dpi("600", false).unwrap();
        assert_eq!(size.pixels().unwrap(), [300, 150]);
        assert_eq!(size.width, "1");
        size.switch_unit(Unit::Millimeters).unwrap();
        assert_eq!(size.pixels().unwrap(), [300, 150]);
        size.width = "0".into();
        assert!(size.pixels().is_err());
        size.width = "NaN".into();
        assert!(size.pixels().is_err());
        size.unit = Unit::Pixels;
        size.width = "1.5".into();
        assert!(size.pixels().is_err());
        size.width = "1".into();
        size.height = "1".into();
        size.dpi = "0".into();
        assert!(size.pixels().is_err());
    }
    #[test]
    fn presets_restart_import_backup_damage_and_failed_save_preserve_previous() {
        let root = TempDir::new();
        let storage = UserStorage::at(root.0.join("userdata")).unwrap();
        let mut book = PresetBook::load(&storage).unwrap();
        book.save(&storage).unwrap();
        let before = book.clone();
        book.entries
            .push(Preset::from_dimensions("Custom".into(), &Dimensions::new(3, 2, 144.0)).unwrap());
        book.save(&storage).unwrap();
        assert_eq!(PresetBook::load(&storage).unwrap(), book);
        assert_eq!(
            PresetBook::read(&storage.root().join("settings/canvas-presets.previous.json"))
                .unwrap(),
            before
        );
        let path = root.0.join("portable.json");
        book.export(&path).unwrap();
        assert_eq!(PresetBook::read(&path).unwrap(), book);
        fs::write(
            storage
                .root()
                .join("settings/canvas-presets.json.write.tmp"),
            "foreign",
        )
        .unwrap();
        assert!(before.save(&storage).is_err());
        assert_eq!(PresetBook::load(&storage).unwrap(), book);
        book.entries.push(book.entries[0].clone());
        assert!(!book.valid());
        assert!(book.export(&path).is_err());
        fs::write(&path, "damaged").unwrap();
        assert!(PresetBook::read(&path).is_err());
    }
}
