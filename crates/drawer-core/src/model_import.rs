//! Frozen, bounded local model attributes. No network fetching or source writes.
use crate::{
    model_mesh::{Asset, Surface},
    world_math::{Matrix, V},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub png: String,
}
impl Image {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Model image exceeds8MiB".into());
        }
        let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
        if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
            return Err("Model textures require PNG or JPEG".into());
        }
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let p = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
        use image::ImageEncoder;
        let mut png = vec![];
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                p.as_raw(),
                p.width(),
                p.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            width: p.width() as u16,
            height: p.height() as u16,
            png: STANDARD.encode(png),
        })
    }
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        if self.width == 0
            || self.height == 0
            || self.width > 1024
            || self.height > 1024
            || self.png.len() > 6 * 1024 * 1024
        {
            return Err("Invalid embedded model image".into());
        }
        let bytes = STANDARD.decode(&self.png).map_err(|e| e.to_string())?;
        if bytes.len() < 29 || bytes[24] != 8 || bytes[25] != 6 {
            return Err("Expected an embedded RGBA8 PNG".into());
        }
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| e.to_string())?;
        if image.width() != self.width as u32 || image.height() != self.height as u32 {
            return Err("Embedded image dimensions differ".into());
        }
        Ok(image.to_rgba8().into_raw())
    }

    pub fn bytes(&self) -> usize {
        self.png.len() + self.width as usize * self.height as usize * 4
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wrap {
    Repeat,
    Clamp,
    Mirror,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alpha {
    Opaque,
    Mask,
    Blend,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub image: Option<Image>,
    pub wrap: [Wrap; 2],
    pub flip_v: bool,
    pub alpha: Alpha,
    pub cutoff: f64,
}
impl Material {
    pub fn valid(&self) -> bool {
        self.cutoff.is_finite()
            && (0. ..=1.).contains(&self.cutoff)
            && self.image.as_ref().is_none_or(|i| i.decode().is_ok())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skin {
    pub names: Vec<String>,
    pub parents: Vec<Option<u16>>,
    pub world: Vec<Matrix>,
    pub inverse_bind: Vec<Matrix>,
    pub positions: Vec<V>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f64; 4]>,
}
impl Skin {
    pub fn valid(&self, vertices: usize) -> bool {
        let n = self.names.len();
        if !(1..=128).contains(&n)
            || self.parents.len() != n
            || self.world.len() != n
            || self.inverse_bind.len() != n
            || self.positions.len() != vertices
            || self.joints.len() != vertices
            || self.weights.len() != vertices
        {
            return false;
        }
        if self
            .names
            .iter()
            .any(|n| n.is_empty() || n.len() > 1024 || n.chars().any(char::is_control))
            || self
                .world
                .iter()
                .chain(&self.inverse_bind)
                .any(|m| !m.valid())
            || self
                .positions
                .iter()
                .flatten()
                .any(|n| !n.is_finite() || n.abs() > 1e9)
        {
            return false;
        }
        for i in 0..n {
            let mut p = self.parents[i];
            let mut count = 0;
            while let Some(j) = p {
                if j as usize >= n || j as usize == i || count >= n {
                    return false;
                }
                p = self.parents[j as usize];
                count += 1;
            }
        }
        self.joints.iter().zip(&self.weights).all(|(j, w)| {
            j.iter().all(|i| (*i as usize) < n)
                && w.iter().all(|v| v.is_finite() && (0. ..=1.).contains(v))
                && (w.iter().sum::<f64>() - 1.).abs() < 1e-6
        })
    }
    pub fn bytes(&self) -> usize {
        self.positions.len() * 80 + self.names.iter().map(|n| n.len() + 208).sum::<usize>()
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    pub materials: BTreeMap<u16, Material>,
    pub skin: Option<Skin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::model_source::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub morph: Option<crate::model_edit::Morph>,
}
impl Attributes {
    pub fn valid(&self, s: &Surface, vertices: usize) -> bool {
        self.source.as_ref().is_none_or(|s| s.valid())
            && self.morph.as_ref().is_none_or(|m| {
                m.valid() && m.base.len() == vertices && m.normals.len() == s.triangles.len()
            })
            && self.materials.len() <= 256
            && self
                .materials
                .iter()
                .all(|(i, m)| (*i as usize) < s.materials.len() && m.valid())
            && self.skin.as_ref().is_none_or(|r| r.valid(vertices))
    }
    pub fn bytes(&self) -> usize {
        self.source.as_ref().map_or(0, |s| s.bytes())
            + self.morph.as_ref().map_or(0, |m| m.bytes())
            + self
                .materials
                .values()
                .map(|m| m.image.as_ref().map_or(0, Image::bytes) + 64)
                .sum::<usize>()
            + self.skin.as_ref().map_or(0, Skin::bytes)
    }
}
pub(crate) fn local_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() && !crate::font_library::local(path) {
        return Err("Choose a local drive model".into());
    }
    let p = path.canonicalize().map_err(|e| e.to_string())?;
    if !p.is_file() || !crate::font_library::local(&p) {
        return Err("Choose a local regular model file".into());
    }
    Ok(p)
}
/// Percent decoding happens before path checks; an encoded traversal cannot bypass them.
pub(crate) fn uri_name(uri: &str) -> Result<String, String> {
    let mut bytes = vec![];
    let s = uri.as_bytes();
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' {
            if i + 2 >= s.len() {
                return Err("Invalid percent escape in model URI".into());
            }
            let h = std::str::from_utf8(&s[i + 1..i + 3]).map_err(|_| "Invalid URI escape")?;
            bytes.push(u8::from_str_radix(h, 16).map_err(|_| "Invalid URI escape")?);
            i += 3;
        } else {
            bytes.push(s[i]);
            i += 1;
        }
    }
    let name = String::from_utf8(bytes).map_err(|_| "Model URI must be UTF-8")?;
    if !crate::model_mesh::relative(&name)
        || name
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        || name.contains(['?', '#'])
    {
        return Err("Model URI must remain a relative local model-folder file".into());
    }
    Ok(name)
}
pub(crate) fn resolve(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !crate::model_mesh::relative(name) {
        return Err("Invalid model-folder reference".into());
    }
    let p = local_path(&root.join(name))?;
    if !p.starts_with(root) {
        return Err("Model reference escapes its folder".into());
    }
    Ok(p)
}
pub fn load(path: &Path, budget: usize, cancel: &AtomicBool) -> Result<Asset, String> {
    crate::book_io::check_cancel(cancel)?;
    let path = local_path(path)?;
    let ext = path
        .extension()
        .and_then(|p| p.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut asset = match ext.as_str() {
        "stl" => crate::model_stl::load(&path, budget, cancel)?,
        "fbx" => crate::model_fbx::load(&path, budget, cancel)?,
        "vrm" | "gltf" | "glb" => crate::model_gltf::load(&path, budget, cancel)?,
        "obj" => {
            let mut a = crate::model_obj::load(&path, budget, cancel)?;
            a.warnings.retain(|s| {
                s != "Texture references are retained; current output uses wireframe color"
            });
            let root = path.parent().unwrap();
            let mut used = 0usize;
            for model in &mut a.models {
                let mut attrs = Attributes::default();
                for (i, m) in model.surface.materials.iter().enumerate() {
                    crate::book_io::check_cancel(cancel)?;
                    let Some(name) = &m.texture else { continue };
                    let present = model
                        .surface
                        .references
                        .iter()
                        .any(|r| r.texture && r.present && r.name == *name);
                    if !present {
                        continue;
                    }
                    let p = resolve(root, name)?;
                    let bytes = crate::book_io::read_local(&p, (8 * 1024 * 1024).min(budget / 8))?;
                    let image = Image::from_bytes(&bytes)?;
                    used = used.saturating_add(image.bytes());
                    if used.saturating_mul(8) > budget {
                        return Err("Imported texture reservation exceeds memory budget".into());
                    }
                    attrs.materials.insert(
                        i as u16,
                        Material {
                            image: Some(image),
                            wrap: [Wrap::Repeat; 2],
                            flip_v: true,
                            alpha: Alpha::Blend,
                            cutoff: 0.5,
                        },
                    );
                }
                if !attrs.materials.is_empty() {
                    model.surface.imported = Some(attrs);
                }
            }
            crate::model_source::obj(&path, &mut a)?;
            a.warning("OBJ axes/units are unspecified; use import rotation and scale (Fit changes units). Diffuse PNG/JPEG maps use nearest repeat sampling; advanced MTL statements are listed separately".into());
            a
        }
        _ => return Err("Choose a local OBJ, glTF2, GLB2, VRM, FBX or STL model".into()),
    };
    if !asset.valid() {
        return Err("Imported attributes failed validation".into());
    }
    let bytes = asset
        .models
        .iter()
        .map(|m| m.mesh.vertices.len() * 24 + m.mesh.edges.len() * 16 + m.surface.bytes())
        .sum::<usize>();
    if bytes.saturating_mul(8) > budget {
        return Err("Imported geometry, textures and skin exceed memory reservation".into());
    }
    if asset.models.iter().any(|m| {
        m.surface
            .imported
            .as_ref()
            .is_some_and(|a| a.materials.values().any(|m| m.alpha == Alpha::Blend))
    }) {
        asset.warning("Transparent imported materials use depth-composited surfaces within continuous model groups; 2D layers split groups. Advanced source shading is not reproduced".into());
    }
    crate::book_io::check_cancel(cancel)?;
    Ok(asset)
}
