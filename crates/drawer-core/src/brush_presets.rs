//! Named offline brush definitions with explicit owned procedural dependencies.
use crate::{
    assets::BrushTip,
    tool_properties::{Property, Tool},
    workspaces::Snapshot,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::{self, Read, Write},
    path::Path,
};
const LIMIT: u64 = 65536;
fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name == name.trim()
        && name.chars().count() <= 64
        && !name.chars().any(char::is_control)
}
fn dependencies(tip: BrushTip) -> (&'static str, &'static str) {
    match tip {
        BrushTip::Pen => ("drawer.round.1", "none"),
        BrushTip::Pencil => ("drawer.pencil.1", "drawer.grain.1"),
        BrushTip::Eraser => ("drawer.eraser.1", "none"),
        BrushTip::Wash => ("drawer.airbrush.1", "none"),
        BrushTip::Marker => ("drawer.marker.1", "none"),
        BrushTip::Smudge => ("drawer.smudge.1", "none"),
        BrushTip::PixelPen => ("drawer.pixel.1", "none"),
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub version: u32,
    pub name: String,
    pub tip: BrushTip,
    pub tip_asset: String,
    pub texture: String,
    pub values: [f64; 7],
    pub pressure: crate::pressure::Mapping,
    pub ends: crate::stroke_ends::Settings,
    pub mouse: crate::mouse_pressure::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_material::Settings::is_default"
    )]
    pub material: crate::brush_material::Settings,
    #[serde(default, skip_serializing_if = "crate::texture::Pattern::is_default")]
    pub overlay: crate::texture::Pattern,
    #[serde(
        default,
        skip_serializing_if = "crate::tip_profile::Settings::is_default"
    )]
    pub profile: crate::tip_profile::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::stroke_filter::Settings::is_default"
    )]
    pub filters: crate::stroke_filter::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::tip_sequence::Settings::is_default"
    )]
    pub sequence: crate::tip_sequence::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_dynamics::Settings::is_default"
    )]
    pub dynamics: crate::brush_dynamics::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_pattern::Settings::is_default"
    )]
    pub pattern: crate::brush_pattern::Settings,
    #[serde(
        default,
        skip_serializing_if = "crate::brush_variation::Settings::is_default"
    )]
    pub variation: crate::brush_variation::Settings,
}
impl Preset {
    pub fn material_refs(&self) -> Vec<(u32, crate::material_library::Kind)> {
        crate::tip_texture::references(&self.sequence, &self.variation.dual.controls, self.overlay)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || !valid_name(&self.name) {
            return Err("Invalid preset version/name (1–64 characters, no controls)".into());
        }
        let (tip, texture) = dependencies(self.tip);
        if self.tip_asset != tip || self.texture != texture {
            return Err(
                "Missing or unsupported local tip/texture dependency; previous brush preserved"
                    .into(),
            );
        }
        if Property::ALL.into_iter().zip(self.values).any(|(p, v)| {
            let [a, b] = p.bounds();
            !v.is_finite() || v < a || v > b
        }) || self.pressure.error().is_some()
            || !self.ends.valid()
            || !self.mouse.valid()
            || !self.material.valid()
            || !self.overlay.valid()
            || !self.sequence.valid()
            || !self.dynamics.valid()
            || !self.pattern.valid()
            || !self.variation.valid()
            || !self.filters.valid()
            || !self.profile.valid()
        {
            return Err("Invalid brush preset values".into());
        }
        Ok(())
    }
    pub fn capture(snapshot: &Snapshot, tip: BrushTip, name: &str) -> Result<Self, String> {
        let settings = snapshot
            .properties
            .as_ref()
            .ok_or("Initialize tool properties before capturing a brush")?;
        if !snapshot.valid() || settings.version != 2 {
            return Err("Upgrade and validate settings before capturing a brush".into());
        }
        let n = BrushTip::ALL.iter().position(|t| *t == tip).unwrap();
        let (a, b) = dependencies(tip);
        let p = Self {
            version: 1,
            name: name.trim().into(),
            tip,
            tip_asset: a.into(),
            texture: b.into(),
            values: Property::ALL.map(|p| settings.tools[&Tool::brush(tip)][&p].value),
            pressure: snapshot.pressure.tools[n],
            ends: snapshot.input.ends[n],
            mouse: snapshot.input.mouse_pressure[n],
            material: snapshot.input.materials[n],
            overlay: snapshot.input.textures[n],
            profile: snapshot.input.profiles[n],
            filters: snapshot.input.filters[n],
            sequence: snapshot.input.sequences[n],
            dynamics: snapshot.input.dynamics[n],
            pattern: snapshot.input.patterns[n],
            variation: snapshot.input.variations[n],
        };
        p.validate()?;
        Ok(p)
    }
    pub fn apply(&self, current: &Snapshot) -> Result<Snapshot, String> {
        self.validate()?;
        if !current.valid() {
            return Err("Invalid current workspace".into());
        }
        let mut next = current.clone();
        let settings = next
            .properties
            .as_mut()
            .ok_or("Initialize tool properties before applying a brush")?;
        if settings.version != 2 {
            return Err("Upgrade tool properties before applying a brush".into());
        }
        for (p, value) in Property::ALL.into_iter().zip(self.values) {
            settings
                .tools
                .get_mut(&Tool::brush(self.tip))
                .unwrap()
                .get_mut(&p)
                .unwrap()
                .value = value;
        }
        let n = BrushTip::ALL.iter().position(|t| *t == self.tip).unwrap();
        next.panels.brush_size = self.values[0];
        next.panels.brush_aspect = self.values[1];
        next.panels.brush_angle = self.values[2];
        next.input.smoothing[n] = self.values[3];
        next.panels.brush_opacity[n] = self.values[4];
        next.panels.brush_hardness[n] = self.values[5];
        next.panels.brush_spacing[n] = self.values[6];
        next.pressure.tools[n] = self.pressure;
        next.input.ends[n] = self.ends;
        next.input.mouse_pressure[n] = self.mouse;
        next.input.materials[n] = self.material;
        next.input.textures[n] = self.overlay;
        next.input.profiles[n] = self.profile;
        next.input.filters[n] = self.filters;
        next.input.sequences[n] = self.sequence;
        next.input.dynamics[n] = self.dynamics;
        next.input.patterns[n] = self.pattern;
        next.input.variations[n] = self.variation;
        next.tool = self.tip.name().into();
        if !next.valid() {
            return Err("Preset would create invalid workspace settings".into());
        }
        Ok(next)
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate().map_err(io::Error::other)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::Error::other("Brush preset exceeds 64 KiB"));
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() as u64 > LIMIT {
            return Err(io::Error::other("Brush preset exceeds 64 KiB"));
        }
        let p: Self = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        p.validate().map_err(io::Error::other)?;
        Ok(p)
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("Choose a regular local preset file"));
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::Error::other("Brush preset exceeds 64 KiB"));
        }
        Self::decode(&bytes)
    }
    /// false creates atomically without replacing a concurrently created destination.
    pub fn save(&self, path: &Path, overwrite: bool) -> io::Result<()> {
        let bytes = self.encode()?;
        if !overwrite && path.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Confirm replacing this preset first",
            ));
        }
        let tmp = path.with_extension("brush-write.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let write = (|| {
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        drop(file);
        let result = write.and_then(|_| {
            if overwrite {
                std::fs::rename(&tmp, path)
            } else {
                std::fs::hard_link(&tmp, path)
            }
        });
        if !overwrite || result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Book {
    pub version: u32,
    pub presets: Vec<Preset>,
    pub sizes: Vec<f64>,
}
impl Default for Book {
    fn default() -> Self {
        Self {
            version: 1,
            presets: vec![],
            sizes: vec![1.0, 2.0, 4.0, 8.0, 12.0, 24.0, 48.0],
        }
    }
}
impl Book {
    pub fn valid(&self) -> bool {
        let mut names = HashSet::new();
        self.version == 1
            && self.presets.len() <= 128
            && self.sizes.len() <= 128
            && self
                .presets
                .iter()
                .all(|p| p.validate().is_ok() && names.insert(name_key(&p.name)))
            && self
                .sizes
                .iter()
                .enumerate()
                .all(|(n, v)| size_valid(*v) && !self.sizes[..n].iter().any(|p| same_size(*p, *v)))
    }
    pub fn contains(&self, name: &str) -> bool {
        self.presets
            .iter()
            .any(|p| name_key(&p.name) == name_key(name))
    }
    pub fn put(&mut self, preset: Preset, overwrite: bool) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid brush collection".into());
        }
        preset.validate()?;
        if let Some(n) = self
            .presets
            .iter()
            .position(|p| name_key(&p.name) == name_key(&preset.name))
        {
            if !overwrite {
                return Err("A preset with this name exists; confirm replacement".into());
            }
            self.presets[n] = preset;
        } else {
            if self.presets.len() >= 128 {
                return Err("Up to 128 local presets".into());
            }
            self.presets.push(preset);
        }
        Ok(())
    }
    pub fn add_size(&mut self, value: f64) -> Result<usize, String> {
        if !self.valid() || !size_valid(value) {
            return Err("Enter a brush size from 0.1 to 4096 px".into());
        }
        if let Some(n) = self.sizes.iter().position(|v| same_size(*v, value)) {
            return Ok(n);
        }
        if self.sizes.len() >= 128 {
            return Err("Up to 128 quick brush sizes".into());
        }
        self.sizes.push(value);
        Ok(self.sizes.len() - 1)
    }
    pub fn remove_size(&mut self, n: usize) -> bool {
        if n >= self.sizes.len() {
            false
        } else {
            self.sizes.remove(n);
            true
        }
    }
    pub fn move_size(&mut self, n: usize, delta: isize) -> bool {
        let Some(to) = n.checked_add_signed(delta) else {
            return false;
        };
        if n >= self.sizes.len() || to >= self.sizes.len() {
            false
        } else {
            self.sizes.swap(n, to);
            true
        }
    }
}
fn size_valid(v: f64) -> bool {
    v.is_finite() && (0.1..=4096.0).contains(&v)
}
fn same_size(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0)
}
pub fn select_size(current: &Snapshot, tip: BrushTip, value: f64) -> Result<Snapshot, String> {
    if !size_valid(value) || !current.valid() || current.tool != tip.name() {
        return Err("Invalid brush size/current workspace".into());
    }
    let mut next = current.clone();
    let settings = next
        .properties
        .as_mut()
        .ok_or("Initialize tool properties before selecting size")?;
    if settings.version != 2 {
        return Err("Upgrade tool properties before selecting size".into());
    }
    settings
        .tools
        .get_mut(&Tool::brush(tip))
        .unwrap()
        .get_mut(&Property::Size)
        .unwrap()
        .value = value;
    next.panels.brush_size = value;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        }
    }
    #[test]
    fn capture_restore_and_size_selection_preserve_other_values_and_dependencies() {
        let mut current = snapshot();
        current
            .properties
            .as_mut()
            .unwrap()
            .tools
            .get_mut(&Tool::Pencil)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap()
            .value = 47.0;
        current.input.mouse_pressure[1].seed = 73;
        current.input.ends[1] = crate::stroke_ends::Settings {
            method: Default::default(),
            fade_minimum: 0.0,
            start: 2.0,
            end: 7.0,
        };
        current.pressure.tools[1].opacity = true;
        current.input.materials[1] = crate::brush_material::Settings {
            mode: crate::brush_material::Mode::Water,
            supply: 0.2,
            retention: 0.8,
            keep_density: true,
            ..Default::default()
        };
        let p = Preset::capture(&current, BrushTip::Pencil, "연필 질감").unwrap();
        assert_eq!(p.texture, "drawer.grain.1");
        let restored = p.apply(&snapshot()).unwrap();
        assert_eq!(
            restored.properties.as_ref().unwrap().tools[&Tool::Pencil][&Property::Size].value,
            47.0
        );
        assert_eq!(restored.input.ends[1], current.input.ends[1]);
        assert_eq!(restored.pressure.tools[1], current.pressure.tools[1]);
        assert_eq!(restored.input.mouse_pressure[1].seed, 73);
        assert_eq!(restored.input.materials[1], current.input.materials[1]);
        assert_eq!(
            Preset::decode(&p.encode().unwrap()).unwrap().material,
            p.material
        );
        let before = snapshot();
        for size in [0.1, 4096.0] {
            let next = select_size(&before, BrushTip::Pen, size).unwrap();
            let mut expected = before.clone();
            expected.panels.brush_size = size;
            expected
                .properties
                .as_mut()
                .unwrap()
                .tools
                .get_mut(&Tool::Pen)
                .unwrap()
                .get_mut(&Property::Size)
                .unwrap()
                .value = size;
            assert_eq!(next, expected);
        }
        for bad in [0.0, f64::NAN, 4097.0] {
            assert!(select_size(&before, BrushTip::Pen, bad).is_err());
        }
        let mut missing = p.clone();
        missing.tip_asset = "missing-tip".into();
        assert!(missing.apply(&before).is_err());
        missing = p;
        missing.texture = "missing-grain".into();
        assert!(missing.apply(&before).is_err());
        assert_eq!(before, snapshot());
    }
    #[test]
    fn named_duplicates_limits_order_and_legacy_workspace_default() {
        let mut book = Book::default();
        let p = Preset::capture(&snapshot(), BrushTip::Pen, "Ink").unwrap();
        book.put(p.clone(), false).unwrap();
        let before = book.clone();
        let mut replacement = p;
        replacement.name = "ink".into();
        replacement.values[0] = 31.0;
        assert!(book.put(replacement.clone(), false).is_err());
        assert_eq!(book, before);
        book.put(replacement, true).unwrap();
        assert_eq!(book.presets.len(), 1);
        assert_eq!(book.presets[0].values[0], 31.0);
        assert_eq!(book.add_size(12.0).unwrap(), 4);
        assert_eq!(book.sizes.len(), 7);
        let n = book.add_size(0.1).unwrap();
        assert!(book.move_size(n, -1));
        assert_eq!(book.sizes[n - 1], 0.1);
        assert!(book.remove_size(n - 1));
        assert!(!book.remove_size(999));
        assert!(!book.move_size(0, -1));
        let mut old = serde_json::to_value(snapshot()).unwrap();
        old.as_object_mut().unwrap().remove("brushes");
        assert_eq!(
            serde_json::from_value::<Snapshot>(old).unwrap().brushes,
            Book::default()
        );
    }
    #[test]
    fn collection_and_size_limits_reject_without_partial_changes() {
        let mut book = Book {
            sizes: vec![],
            ..Default::default()
        };
        for n in 0..128 {
            book.add_size(n as f64 + 0.1).unwrap();
        }
        let before = book.clone();
        assert!(book.add_size(4000.0).is_err());
        assert_eq!(book, before);
        assert_eq!(book.add_size(1.1).unwrap(), 1);
        assert_eq!(book, before);
        let mut p = Preset::capture(&snapshot(), BrushTip::Pen, "Ink").unwrap();
        for n in 0..128 {
            p.name = format!("Brush {n}");
            book.put(p.clone(), false).unwrap();
        }
        let before = book.clone();
        p.name = "Too many".into();
        assert!(book.put(p.clone(), false).is_err());
        assert_eq!(book, before);
        p.name = "Brush 0".into();
        p.values[0] = 23.0;
        book.put(p, true).unwrap();
        assert_eq!(book.presets.len(), 128);
        assert_eq!(book.presets[0].values[0], 23.0);
    }
    #[test]
    fn local_file_roundtrip_explicit_overwrite_corruption_and_foreign_temporary() {
        let dir = crate::test_support::TempDir::new();
        let path = dir.0.join("연필.json");
        let p = Preset::capture(&snapshot(), BrushTip::Pencil, "연필").unwrap();
        p.save(&path, false).unwrap();
        assert_eq!(Preset::read(&path).unwrap(), p);
        assert!(!path.with_extension("brush-write.tmp").exists());
        let bytes = std::fs::read(&path).unwrap();
        let mut changed = p.clone();
        changed.values[0] = 77.0;
        assert!(changed.save(&path, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        changed.save(&path, true).unwrap();
        assert_eq!(Preset::read(&path).unwrap(), changed);
        let foreign = path.with_extension("brush-write.tmp");
        std::fs::write(&foreign, "foreign").unwrap();
        assert!(p.save(&path, true).is_err());
        assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "foreign");
        assert_eq!(Preset::read(&path).unwrap(), changed);
        std::fs::write(&path, "broken").unwrap();
        assert!(Preset::read(&path).is_err());
        std::fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Preset::read(&path).is_err());
    }
    #[test]
    fn overlay_recipe_missing_reference_and_legacy_preset_defaults() {
        let mut before = snapshot();
        before.input.textures[0] = crate::texture::Pattern {
            source: 999,
            ..Default::default()
        };
        let p = Preset::capture(&before, BrushTip::Pen, "Missing recipe").unwrap();
        let decoded = Preset::decode(&p.encode().unwrap()).unwrap();
        let applied = decoded.apply(&snapshot()).unwrap();
        assert!(applied.input.textures[0].missing());
        assert_eq!(applied.input.textures[0].factor([3.5, 4.5]), 1.0);
        let mut old = serde_json::to_value(&p).unwrap();
        old.as_object_mut().unwrap().remove("overlay");
        let legacy: Preset = serde_json::from_value(old).unwrap();
        assert!(legacy.overlay.is_default());
    }
}
