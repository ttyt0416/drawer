//! Bounded offline raster mask library. Source images are read-only snapshots.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{self, Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
};
pub const FIRST_ID: u32 = 1024;
const MAX_MASK: usize = 512 * 512;
const MAX_STORED: usize = 2 * 1024 * 1024;
const MAX_FILE: u64 = 8 * 1024 * 1024;
const DECODE_BUDGET: usize = 32 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Tip,
    Texture,
}
impl Kind {
    pub const ALL: [Self; 2] = [Self::Tip, Self::Texture];
    pub fn label(self) -> &'static str {
        match self {
            Self::Tip => "Brush tip mask",
            Self::Texture => "Brush texture",
        }
    }
}
fn bounded<'de, D, T: Deserialize<'de>, const N: usize>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "at most {N} material items")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(value) = seq.next_element()? {
                if result.len() == N {
                    return Err(serde::de::Error::custom("Material allocation limit"));
                }
                result.push(value);
            }
            Ok(result)
        }
    }
    d.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
}
fn bytes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    bounded::<D, u8, MAX_MASK>(d)
}
fn entries<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Asset>, D::Error> {
    bounded::<D, Asset, 32>(d)
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn valid_name(s: &str) -> bool {
    !s.trim().is_empty()
        && s == s.trim()
        && s.chars().count() <= 64
        && !s.chars().any(char::is_control)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    #[default]
    DarknessOrAlpha,
    LumaOrAlpha,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: u32,
    pub name: String,
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    #[serde(deserialize_with = "bytes")]
    pub values: Vec<u8>,
    #[serde(default)]
    pub density: Density,
    #[serde(default)]
    pub fibres: Option<crate::fibres::Set>,
}
impl Asset {
    pub(crate) fn valid(&self) -> bool {
        self.id >= FIRST_ID
            && valid_name(&self.name)
            && (1..=512).contains(&self.width)
            && (1..=512).contains(&self.height)
            && match &self.fibres {
                Some(set) => self.kind == Kind::Tip && self.values.is_empty() && set.valid(),
                None => {
                    self.values.len() == self.width as usize * self.height as usize
                        && (self.kind != Kind::Tip
                            || self.density == Density::LumaOrAlpha
                            || self.values.iter().any(|v| *v > 0))
                }
            }
    }
    pub fn tip(&self, q: [f64; 2]) -> f64 {
        if q.iter().any(|n| !n.is_finite() || n.abs() > 1.0) {
            return 0.0;
        }
        if let Some(set) = &self.fibres {
            return set.sample(q, self.width.max(self.height));
        }
        let p = [
            (q[0] + 1.0) * self.width as f64 * 0.5 - 0.5,
            (q[1] + 1.0) * self.height as f64 * 0.5 - 0.5,
        ];
        let low = p.map(|v| v.floor() as i64);
        let t = [p[0] - low[0] as f64, p[1] - low[1] as f64];
        let mut sum = 0.0;
        for y in 0..2 {
            for x in 0..2 {
                let sx = (low[0] + x as i64).clamp(0, self.width as i64 - 1);
                let sy = (low[1] + y as i64).clamp(0, self.height as i64 - 1);
                let w =
                    if x == 0 { 1.0 - t[0] } else { t[0] } * if y == 0 { 1.0 - t[1] } else { t[1] };
                sum += self.values[(sy * self.width as i64 + sx) as usize] as f64 / 255.0 * w;
            }
        }
        sum
    }
    pub fn sample_cost(&self) -> u64 {
        self.fibres
            .as_ref()
            .map_or(4, |s| s.fibres.len() as u64 + 1)
    }
    pub fn tile(&self, point: [f64; 2]) -> f64 {
        // A tiny negative coordinate can round its floating remainder to exactly
        // the positive divisor. It belongs to the last texel, never one past it.
        let x =
            (point[0].rem_euclid(self.width as f64).floor() as usize).min(self.width as usize - 1);
        let y = (point[1].rem_euclid(self.height as f64).floor() as usize)
            .min(self.height as usize - 1);
        self.values[y * self.width as usize + x] as f64 / 255.0
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    version: u32,
    next_id: u32,
    #[serde(deserialize_with = "entries")]
    assets: Vec<Asset>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: FIRST_ID,
            assets: vec![],
        }
    }
}
impl Library {
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }
    pub fn get(&self, id: u32, kind: Kind) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id && a.kind == kind)
    }
    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        self.version == 1
            && self.next_id >= FIRST_ID
            && self.assets.len() <= 32
            && self.assets.iter().map(|a| a.values.len()).sum::<usize>() <= MAX_STORED
            && self.assets.iter().all(|a| {
                a.valid()
                    && a.id < self.next_id
                    && ids.insert(a.id)
                    && names.insert(a.name.to_lowercase())
            })
    }
    pub fn label(&self, id: u32, kind: Kind) -> Option<String> {
        self.get(id, kind)
            .map(|a| format!("{} (#{}): {}×{}", a.name, a.id, a.width, a.height))
    }
    pub fn import(&self, path: &Path, name: &str, kind: Kind) -> io::Result<Self> {
        self.import_density(path, name, kind, Density::DarknessOrAlpha)
    }
    pub fn import_density(
        &self,
        path: &Path,
        name: &str,
        kind: Kind,
        density: Density,
    ) -> io::Result<Self> {
        if !self.valid() {
            return Err(invalid("Existing material library is invalid"));
        }
        let name = name.trim();
        if !valid_name(name)
            || self
                .assets
                .iter()
                .any(|a| a.name.to_lowercase() == name.to_lowercase())
        {
            return Err(invalid(
                "Use a unique material name (1–64 characters, no controls)",
            ));
        }
        let cancel = AtomicBool::new(false);
        let source = crate::image_import::ImageSource::read(path, DECODE_BUDGET, &cancel)?;
        let format =
            image::guess_format(&source.encoded_snapshot()).map_err(|e| invalid(e.to_string()))?;
        if !matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Tiff
        ) || source.pages.len() != 1
        {
            return Err(invalid(
                "Materials support still PNG, JPEG and single-page TIFF",
            ));
        }
        let info = &source.pages[0];
        if !(1..=512).contains(&info.width) || !(1..=512).contains(&info.height) {
            return Err(invalid("Material image must be 1–512 pixels on each side"));
        }
        let page = source.decode(0, true, DECODE_BUDGET, &cancel)?;
        let image = page.image;
        if density == Density::DarknessOrAlpha && image.pixels.iter().all(|p| p[3] == 0) {
            return Err(invalid(
                "Completely transparent material image has no usable pixels",
            ));
        }
        let alpha = image.pixels.iter().any(|p| p[3] != 255);
        let values = image
            .pixels
            .iter()
            .map(|p| {
                let luma = (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19 + 128) / 256;
                match kind {
                    Kind::Tip => {
                        if alpha {
                            p[3]
                        } else {
                            if density == Density::LumaOrAlpha {
                                luma as u8
                            } else {
                                (255 - luma) as u8
                            }
                        }
                    }
                    Kind::Texture => {
                        ((luma * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255) as u8
                    }
                }
            })
            .collect();
        self.register_asset(
            Asset {
                id: FIRST_ID,
                name: name.into(),
                kind,
                width: image.width,
                height: image.height,
                values,
                density,
                fibres: None,
            },
            false,
        )
    }
    /// Register independently decoded mask bytes; validation and publication stay separate.
    pub fn register_mask(
        &self,
        name: &str,
        kind: Kind,
        width: u32,
        height: u32,
        values: Vec<u8>,
    ) -> io::Result<Self> {
        self.register_asset(
            Asset {
                id: FIRST_ID,
                name: name.trim().into(),
                kind,
                width,
                height,
                values,
                density: Density::default(),
                fibres: None,
            },
            false,
        )
    }
    pub fn register_fibres(&self, name: &str, set: crate::fibres::Set) -> io::Result<Self> {
        self.register_asset(
            Asset {
                id: FIRST_ID,
                name: name.trim().into(),
                kind: Kind::Tip,
                width: 128,
                height: 128,
                values: vec![],
                density: Density::default(),
                fibres: Some(set),
            },
            false,
        )
    }
    /// Always assigns a fresh local ID. Bundle collisions receive a display suffix.
    pub(crate) fn register_asset(&self, mut asset: Asset, rename: bool) -> io::Result<Self> {
        if !self.valid() || !asset.valid() {
            return Err(invalid("Invalid material/library"));
        }
        let original = asset.name.clone();
        let mut n = 2;
        while self
            .assets
            .iter()
            .any(|a| a.name.to_lowercase() == asset.name.to_lowercase())
        {
            if !rename {
                return Err(invalid("Duplicate material name"));
            }
            let tail = format!(" ({n})");
            asset.name = format!(
                "{}{}",
                original.chars().take(64 - tail.len()).collect::<String>(),
                tail
            );
            n += 1;
        }
        let mut next = self.clone();
        asset.id = next.next_id;
        next.next_id = next
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Material ID limit"))?;
        next.assets.push(asset);
        if !next.valid() {
            return Err(invalid(
                "Material capacity exceeded (32 items / 2 MiB masks)",
            ));
        }
        Ok(next)
    }
    pub fn remove(&self, id: u32) -> io::Result<Self> {
        let mut next = self.clone();
        let n = next.assets.len();
        next.assets.retain(|a| a.id != id);
        if n == next.assets.len() {
            return Err(invalid("Material is unavailable"));
        }
        Ok(next)
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut file = fs::File::open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_FILE {
            return Err(invalid("Material library exceeds 8 MiB"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_FILE + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(invalid("Material library grew beyond its limit"));
        }
        let lib: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if !lib.valid() {
            return Err(invalid("Invalid material library"));
        }
        Ok(lib)
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if !self.valid() {
            return Err(invalid("Invalid material library"));
        }
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(invalid("Material library exceeds 8 MiB"));
        }
        let temp = path.with_extension("json.write.tmp");
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        let result = (|| {
            f.write_all(&bytes)?;
            f.sync_all()
        })();
        drop(f);
        if let Err(e) = result.and_then(|_| fs::rename(&temp, path)) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        Ok(())
    }
    pub fn raw_texture(
        &self,
        mut pattern: crate::texture::Pattern,
        point: [f64; 2],
    ) -> Option<f64> {
        if pattern.source == 0
            || (pattern.source > 3 && self.get(pattern.source, Kind::Texture).is_none())
        {
            return None;
        }
        pattern.strength = 1.0;
        Some(self.factor(pattern, point))
    }
    pub fn factor(&self, p: crate::texture::Pattern, point: [f64; 2]) -> f64 {
        if let Some(asset) = self.get(p.source, Kind::Texture) {
            if !p.valid() {
                return 1.0;
            }
            let (s, c) = crate::texture::rotation(p.angle);
            let q = [
                (point[0] - p.offset[0]) / p.scale,
                (point[1] - p.offset[1]) / p.scale,
            ];
            1.0 - p.strength + p.strength * asset.tile([q[0] * c + q[1] * s, -q[0] * s + q[1] * c])
        } else {
            p.factor(point)
        }
    }
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[[u8; 4]]) -> io::Result<()> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    let rgba = pixels.iter().flatten().copied().collect::<Vec<_>>();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(io::Error::other)?;
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?
        .write_all(&bytes)
}
fn canvas() -> io::Result<crate::document::Document> {
    crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 32,
            height: 16,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        65536,
    )
    .map_err(io::Error::other)
}
fn brush(tip: u32, texture: u32) -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: crate::tip_sequence::Settings {
            enabled: true,
            count: 1,
            tips: [tip, 0, 0, 0, 0, 0, 0, 0],
            ..Default::default()
        },
        filters: Default::default(),
        profile: Default::default(),
        texture: crate::texture::Pattern {
            source: texture,
            strength: 1.0,
            ..Default::default()
        },
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 8.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [40, 100, 200, 255],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.5,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: true,
            ..Default::default()
        },
    }
}
/// Validate encoded snapshots, independent local persistence and actual tip/texture pixels.
pub fn verify(root: &Path) -> io::Result<Library> {
    let source = root.join("material-source-alpha.png");
    let texture = root.join("material-source-texture.png");
    write_png(
        &source,
        2,
        2,
        &[[0, 0, 0, 255], [0; 4], [0; 4], [0, 0, 0, 255]],
    )?;
    write_png(
        &texture,
        2,
        2,
        &[[0, 0, 0, 255], [255; 4], [0, 0, 0, 255], [255; 4]],
    )?;
    let original = fs::read(&source)?;
    let library = Library::default()
        .import(&source, "Diagonal tip", Kind::Tip)?
        .import(&texture, "Striped texture", Kind::Texture)?;
    if fs::read(&source)? != original
        || library.assets.len() != 2
        || library.assets[0].values != [255, 0, 0, 255]
    {
        return Err(invalid("Material conversion/source preservation failed"));
    }
    let path = root.join("material-library.json");
    library.save(&path)?;
    let library = Library::read(&path)?;
    let tip = library.assets[0].id;
    let texture = library.assets[1].id;
    let mut d = canvas()?;
    let before = d.snapshot();
    let mut st = crate::painting::Stroke::begin(&d, 0, brush(tip, 0))
        .map_err(io::Error::other)?
        .with_materials(std::sync::Arc::new(library.clone()))
        .map_err(io::Error::other)?;
    st.direct_dab(&d, [12.5, 8.5], [12.5, 8.5], 1.0)
        .map_err(io::Error::other)?;
    if !st.mask_is_square() || d.snapshot() != before {
        return Err(invalid("Bitmap outline/preview source changed"));
    }
    st.commit_mouse(
        &mut d,
        crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: brush(tip, 0),
            count: 1,
            ends: Default::default(),
            native_pressure: true,
        },
    )
    .map_err(io::Error::other)?;
    if d.composite_pixel(9, 5) != [40, 100, 200, 255]
        || d.composite_pixel(15, 5)[3] != 0
        || d.history_counts() != (1, 0)
    {
        return Err(invalid(
            "Actual bitmap tip lost a corner or transparent quadrant",
        ));
    }
    let after = d.snapshot();
    if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
        return Err(invalid("Bitmap history diverged"));
    }
    let native = root.join("material-pixels.drawer");
    crate::files::save(&mut d, &native)?;
    let reopened = crate::files::open(&native, 65536)?;
    if reopened.snapshot() != after {
        return Err(invalid("Native material pixels/references changed"));
    }
    let mut d = canvas()?;
    let mut st = crate::painting::Stroke::begin(&d, 0, brush(0, texture))
        .map_err(io::Error::other)?
        .with_materials(std::sync::Arc::new(library.clone()))
        .map_err(io::Error::other)?;
    st.direct_dab(&d, [12.5, 8.5], [12.5, 8.5], 1.0)
        .map_err(io::Error::other)?;
    st.commit(&mut d).map_err(io::Error::other)?;
    if d.composite_pixel(12, 8)[3] != 0 || d.composite_pixel(13, 8) != [40, 100, 200, 255] {
        return Err(invalid("Registered texture was not used exactly once"));
    }
    Ok(library)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgba_conversion_local_reload_actual_masks_texture_and_history() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn reject_damaged_transparent_empty_giant_or_wrong_format_preserving_materials() {
        let root = crate::test_support::TempDir::new();
        let lib = verify(&root.0).unwrap();
        let before = lib.clone();
        let path = root.0.join("bad.png");
        fs::write(&path, b"broken").unwrap();
        assert!(lib.import(&path, "Bad", Kind::Tip).is_err());
        fs::remove_file(&path).unwrap();
        write_png(&path, 1, 1, &[[0; 4]]).unwrap();
        assert!(lib.import(&path, "Transparent", Kind::Tip).is_err());
        assert!(lib.import(&path, "Transparent", Kind::Texture).is_err());
        fs::remove_file(&path).unwrap();
        write_png(&path, 1, 1, &[[255; 4]]).unwrap();
        assert!(lib.import(&path, "Empty white tip", Kind::Tip).is_err());
        assert!(lib.import(&path, "White texture", Kind::Texture).is_ok());
        fs::remove_file(&path).unwrap();
        write_png(&path, 513, 1, &vec![[0, 0, 0, 255]; 513]).unwrap();
        assert!(
            lib.import(&path, "Too large", Kind::Tip)
                .unwrap_err()
                .to_string()
                .contains("512")
        );
        let zero = root.0.join("zero.png");
        let mut bytes = fs::read(root.0.join("material-source-alpha.png")).unwrap();
        bytes[16..20].copy_from_slice(&0u32.to_be_bytes());
        fs::write(&zero, bytes).unwrap();
        assert!(lib.import(&zero, "Zero", Kind::Tip).is_err());
        assert!(
            lib.import(
                &root.0.join("material-source-alpha.png"),
                "DIAGONAL TIP",
                Kind::Tip
            )
            .is_err()
        );
        assert!(lib == before);
        assert!(Library::read(&root.0.join("material-library.json")).unwrap() == before);
    }
    #[test]
    fn atomic_foreign_temporary_corrupt_library_and_bounded_deserializers() {
        let root = crate::test_support::TempDir::new();
        let lib = verify(&root.0).unwrap();
        let path = root.0.join("material-library.json");
        let original = fs::read(&path).unwrap();
        let temp = path.with_extension("json.write.tmp");
        fs::write(&temp, "foreign").unwrap();
        assert!(lib.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&temp).unwrap(), b"foreign");
        let mut bad = serde_json::to_value(&lib).unwrap();
        bad["assets"][0]["width"] = 0.into();
        fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(Library::read(&path).is_err());
        let mut bad = serde_json::to_value(&lib).unwrap();
        bad["assets"] = serde_json::Value::Array(vec![bad["assets"][0].clone(); 33]);
        assert!(serde_json::from_value::<Library>(bad).is_err());
        let mut bad = serde_json::to_value(&lib).unwrap();
        bad["assets"][0]["values"] = serde_json::to_value(vec![255u8; MAX_MASK + 1]).unwrap();
        assert!(serde_json::from_value::<Library>(bad).is_err());
    }
    #[test]
    fn frozen_library_removal_missing_fallback_and_no_reused_ids() {
        let root = crate::test_support::TempDir::new();
        let lib = verify(&root.0).unwrap();
        let id = lib.assets[0].id;
        let arc = std::sync::Arc::new(lib.clone());
        let mut d = canvas().unwrap();
        let mut st = crate::painting::Stroke::begin(&d, 0, brush(id, 0))
            .unwrap()
            .with_materials(arc)
            .unwrap();
        let removed = lib.remove(id).unwrap();
        st.direct_dab(&d, [12.5, 8.5], [12.5, 8.5], 1.0).unwrap();
        st.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(15, 5)[3], 0);
        let mut next = canvas().unwrap();
        let mut st = crate::painting::Stroke::begin(&next, 0, brush(id, 0))
            .unwrap()
            .with_materials(std::sync::Arc::new(removed.clone()))
            .unwrap();
        st.direct_dab(&next, [12.5, 8.5], [12.5, 8.5], 1.0).unwrap();
        st.commit(&mut next).unwrap();
        assert!(next.composite_pixel(14, 7)[3] > 0);
        assert!(d.layers[0].pixels != next.layers[0].pixels);
        let again = removed
            .import(
                &root.0.join("material-source-alpha.png"),
                "New tip",
                Kind::Tip,
            )
            .unwrap();
        assert!(again.assets.last().unwrap().id > id);
        assert!(again.get(id, Kind::Tip).is_none());
    }
    #[test]
    fn image_coordinates_texture_transform_limits_and_source_independence() {
        let root = crate::test_support::TempDir::new();
        let lib = verify(&root.0).unwrap();
        let tip = &lib.assets[0];
        assert_eq!(tip.tip([-0.5, -0.5]), 1.0);
        assert_eq!(tip.tip([0.5, -0.5]), 0.0);
        assert_eq!(tip.tip([1.1, 0.0]), 0.0);
        let p = crate::texture::Pattern {
            source: lib.assets[1].id,
            strength: 1.0,
            ..Default::default()
        };
        assert_eq!(lib.factor(p, [0.5, 0.5]), 0.0);
        assert_eq!(lib.factor(p, [1.5, 0.5]), 1.0);
        assert_eq!(
            lib.factor(
                crate::texture::Pattern {
                    offset: [1.0, 0.0],
                    ..p
                },
                [0.5, 0.5]
            ),
            1.0
        );
        assert_eq!(
            lib.factor(crate::texture::Pattern { scale: 2.0, ..p }, [1.5, 0.5]),
            0.0
        );
        assert_eq!(
            lib.factor(crate::texture::Pattern { strength: 0.0, ..p }, [0.5, 0.5]),
            1.0
        );
        fs::remove_file(root.0.join("material-source-alpha.png")).unwrap();
        let loaded = Library::read(&root.0.join("material-library.json")).unwrap();
        assert_eq!(loaded.get(tip.id, Kind::Tip).unwrap().values, tip.values);
        let mut invalid = lib;
        invalid.assets[0].id = 1;
        assert!(!invalid.valid());
        let limit = Library {
            next_id: u32::MAX,
            ..Default::default()
        };
        assert!(
            limit
                .import(
                    &root.0.join("material-source-texture.png"),
                    "Last",
                    Kind::Texture
                )
                .is_err()
        );
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    #[test]
    fn jpeg_single_page_tiff_rejection_of_bmp_and_multi_page_tiff() {
        let root = crate::test_support::TempDir::new();
        let jpeg = root.0.join("image.jpg");
        let mut data = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut data, 100)
            .encode(
                &[10, 40, 80, 10, 40, 80],
                2,
                1,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        fs::write(&jpeg, &data).unwrap();
        let lib = Library::default().import(&jpeg, "JPEG", Kind::Tip).unwrap();
        assert_eq!(lib.assets[0].width, 2);
        assert_eq!(fs::read(&jpeg).unwrap(), data);
        let tiff = root.0.join("image.tif");
        {
            let mut writer =
                tiff::encoder::TiffEncoder::new(fs::File::create(&tiff).unwrap()).unwrap();
            writer
                .write_image::<tiff::encoder::colortype::RGB8>(2, 1, &[0, 0, 0, 255, 255, 255])
                .unwrap();
        }
        let lib = lib.import(&tiff, "TIFF", Kind::Texture).unwrap();
        assert_eq!(lib.assets[1].values, [0, 255]);
        let multi = root.0.join("multi.tif");
        {
            let mut writer =
                tiff::encoder::TiffEncoder::new(fs::File::create(&multi).unwrap()).unwrap();
            for _ in 0..2 {
                writer
                    .write_image::<tiff::encoder::colortype::RGB8>(1, 1, &[0, 0, 0])
                    .unwrap();
            }
        }
        assert!(lib.import(&multi, "Multi", Kind::Tip).is_err());
        let bmp = root.0.join("other.bmp");
        image::DynamicImage::new_rgba8(2, 2)
            .save_with_format(&bmp, image::ImageFormat::Bmp)
            .unwrap();
        assert!(
            lib.import(&bmp, "BMP", Kind::Tip)
                .unwrap_err()
                .to_string()
                .contains("PNG")
        );
    }
    #[test]
    fn entry_and_memory_caps_and_preset_reference_round_trip() {
        let root = crate::test_support::TempDir::new();
        let source = root.0.join("one.png");
        write_png(&source, 1, 1, &[[0, 0, 0, 255]]).unwrap();
        let mut lib = Library::default();
        for n in 0..32 {
            lib = lib.import(&source, &format!("Tip {n}"), Kind::Tip).unwrap();
        }
        assert_eq!(lib.assets.len(), 32);
        assert!(lib.import(&source, "Overflow", Kind::Tip).is_err());
        let mut big = Library::default();
        for n in 0..9 {
            big.assets.push(Asset {
                density: Density::default(),
                fibres: None,
                id: FIRST_ID + n,
                name: format!("Big {n}"),
                kind: Kind::Tip,
                width: 512,
                height: 512,
                values: vec![255; MAX_MASK],
            });
        }
        big.next_id = FIRST_ID + 9;
        assert!(!big.valid());
        assert!(big.save(&root.0.join("too-big.json")).is_err());
        let mut s = crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        };
        s.input.sequences[0] = brush(lib.assets[0].id, 0).sequence;
        let preset =
            crate::brush_presets::Preset::capture(&s, crate::assets::BrushTip::Pen, "Custom tip")
                .unwrap();
        let path = root.0.join("preset.json");
        preset.save(&path, false).unwrap();
        let next = crate::brush_presets::Preset::read(&path).unwrap();
        assert_eq!(next.sequence.tips[0], lib.assets[0].id);
    }
}

#[cfg(test)]
mod diffusion_tests {
    use super::*;
    #[test]
    fn registered_texture_masks_final_water_diffusion_once() {
        let root = crate::test_support::TempDir::new();
        let lib = verify(&root.0).unwrap();
        let texture = lib
            .assets
            .iter()
            .find(|a| a.kind == Kind::Texture)
            .unwrap()
            .id;
        let mut d = canvas().unwrap();
        let mut s = brush(0, texture);
        s.material.mode = crate::brush_material::Mode::Water;
        s.material.water = 1.0;
        let mut st = crate::painting::Stroke::begin(&d, 0, s)
            .unwrap()
            .with_materials(std::sync::Arc::new(lib))
            .unwrap();
        st.direct_dab(&d, [12.5, 8.5], [12.5, 8.5], 1.0).unwrap();
        st.commit(&mut d).unwrap();
        assert_eq!(d.composite_pixel(12, 8)[3], 0);
        assert!(d.composite_pixel(13, 8)[3] > 0);
    }
}
