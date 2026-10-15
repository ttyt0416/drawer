use crate::storage::UserStorage;
fn default_opacity() -> [f64; 4] {
    [1.0; 4]
}
fn default_spacing() -> [f64; 4] {
    [0.15; 4]
}
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Brush,
    Color,
    Layers,
    Timeline,
}
impl Kind {
    pub const ALL: [Self; 4] = [Self::Brush, Self::Color, Self::Layers, Self::Timeline];
    pub fn index(self) -> usize {
        match self {
            Self::Brush => 0,
            Self::Color => 1,
            Self::Layers => 2,
            Self::Timeline => 3,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brushes",
            Self::Color => "Color",
            Self::Layers => "Layers",
            Self::Timeline => "Timeline",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bar {
    Hidden,
    Top,
    Bottom,
}
impl Bar {
    pub const ALL: [Self; 3] = [Self::Hidden, Self::Top, Self::Bottom];
    pub fn label(self) -> &'static str {
        match self {
            Self::Hidden => "Hidden",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Thumbnail {
    None,
    Small,
    Medium,
    Large,
}
impl Thumbnail {
    pub const ALL: [Self; 4] = [Self::None, Self::Small, Self::Medium, Self::Large];
    pub fn size(self) -> usize {
        match self {
            Self::None => 0,
            Self::Small => 16,
            Self::Medium => 32,
            Self::Large => 64,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub visible: bool,
    pub thumbnail: Thumbnail,
    pub content: bool,
    pub checker: bool,
    pub properties: Bar,
    pub commands: Bar,
    pub icons: bool,
    pub names: bool,
    pub strokes: bool,
    pub animation_only: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            visible: true,
            thumbnail: Thumbnail::Medium,
            content: false,
            checker: true,
            properties: Bar::Top,
            commands: Bar::Bottom,
            icons: true,
            names: true,
            strokes: false,
            animation_only: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub panels: [Panel; 4],
    pub brush_size: f64,
    #[serde(default = "default_opacity")]
    pub brush_opacity: [f64; 4],
    #[serde(default = "default_opacity")]
    pub brush_hardness: [f64; 4],
    #[serde(default = "default_spacing")]
    pub brush_spacing: [f64; 4],
    #[serde(default = "default_aspect")]
    pub brush_aspect: f64,
    #[serde(default)]
    pub brush_angle: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            panels: [Panel::default(); 4],
            brush_size: 12.0,
            brush_opacity: [1.0, 1.0, 1.0, 0.6],
            brush_hardness: [1.0, 1.0, 1.0, 0.0],
            brush_spacing: [0.15, 0.15, 0.15, 0.1],
            brush_aspect: 1.0,
            brush_angle: 0.0,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.version == 1
            && self.brush_hardness.iter().all(|v| (0.0..=1.0).contains(v))
            && self.brush_spacing.iter().all(|v| (0.01..=10.0).contains(v))
            && self
                .brush_opacity
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.brush_size.is_finite()
            && self.brush_size > 0.0
            && self.brush_size <= 4096.0
            && (0.05..=1.0).contains(&self.brush_aspect)
            && self.brush_angle.is_finite()
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Panel settings too large"));
        }
        let settings: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        if !settings.valid() {
            return Err(io::Error::other("Invalid panel settings"));
        }
        Ok(settings)
    }
    pub fn load(storage: &UserStorage) -> io::Result<Self> {
        match Self::read(&storage.root().join("settings/panels.json")) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(self, storage: &UserStorage) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid panel settings"));
        }
        let path = storage.root().join("settings/panels.json");
        if path.exists() {
            if fs::metadata(&path)?.len() > 65536 {
                return Err(io::Error::other("Panel settings exceed recovery limit"));
            }
            let backup = if Self::read(&path).is_ok() {
                "panels.previous.json"
            } else {
                "panels.damaged.json"
            };
            atomic(
                &storage.root().join("settings").join(backup),
                &fs::read(&path)?,
            )?;
        }
        atomic(&path, &serde_json::to_vec(&self).map_err(io::Error::other)?)
    }
}
fn default_aspect() -> f64 {
    1.0
}
fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("json.write.tmp");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(e) = result.and_then(|_| fs::rename(&temp, path)) {
        let _ = fs::remove_file(temp);
        return Err(e);
    }
    Ok(())
}
