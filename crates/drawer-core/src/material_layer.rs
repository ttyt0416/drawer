//! Parameterized local materials. Bitmap sources are immutable and native files embed RGBA.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    gradient,
    scene::{Affine, PageObject, Point, Rect},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    #[serde(skip)]
    pub pixels: Arc<Vec<[u8; 4]>>,
}
impl Image {
    pub fn valid(&self, missing: bool) -> bool {
        (1..=32768).contains(&self.width)
            && (1..=32768).contains(&self.height)
            && u64::from(self.width) * u64::from(self.height) <= 4 * 1024 * 1024
            && (self.pixels.len() == self.width as usize * self.height as usize
                || missing && self.pixels.is_empty())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Source {
    Solid([u8; 4]),
    Gradient(Box<gradient::Settings>),
    Image(Image),
    Metal(Box<crate::metal_material::Metal>),
}
impl Source {
    pub fn image(&self) -> Option<&Image> {
        match self {
            Self::Image(i) => Some(i),
            Self::Metal(m) => m.texture.as_ref(),
            _ => None,
        }
    }
    fn image_mut(&mut self) -> Option<&mut Image> {
        match self {
            Self::Image(i) => Some(i),
            Self::Metal(m) => m.texture.as_mut(),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub source: Source,
    pub area: Rect,
    /// Local coordinates to preserved original-page coordinates.
    pub transform: Affine,
    pub repeat: [bool; 2],
    #[serde(
        default,
        skip_serializing_if = "crate::material_mapping::Mapping::is_default"
    )]
    pub mapping: crate::material_mapping::Mapping,
}
impl Layer {
    pub fn valid(&self, missing: bool) -> bool {
        self.mapping.valid(self.area)
            && self.transform.valid()
            && self.area.valid()
            && [self.area.min, self.area.max]
                .into_iter()
                .flatten()
                .all(|v| v.abs() <= 1e9)
            && self.bounds().valid()
            && match &self.source {
                Source::Solid(_) => true,
                Source::Gradient(s) => s.valid() && s.shape != gradient::Shape::Freeform,
                Source::Image(i) => i.valid(missing),
                Source::Metal(m) => m.valid(missing),
            }
    }
    pub fn bounds(&self) -> Rect {
        let points = self
            .mapping
            .warp
            .points()
            .into_iter()
            .map(|p| {
                self.transform.map(
                    [0, 1].map(|n| self.area.min[n] + p[n] * (self.area.max[n] - self.area.min[n])),
                )
            })
            .collect::<Vec<_>>();
        Rect {
            min: [0, 1].map(|n| points.iter().map(|p| p[n]).fold(f64::INFINITY, f64::min)),
            max: [0, 1].map(|n| {
                points
                    .iter()
                    .map(|p| p[n])
                    .fold(f64::NEG_INFINITY, f64::max)
            }),
        }
    }
    pub fn bytes(&self) -> usize {
        self.mapping.bytes()
            + match &self.source {
                Source::Image(i) => i.width as usize * i.height as usize * 4,
                Source::Gradient(s) => serde_json::to_vec(s).map_or(0, |b| b.len()),
                Source::Metal(m) => {
                    128 + m
                        .texture
                        .as_ref()
                        .map_or(0, |i| i.width as usize * i.height as usize * 4)
                }
                _ => 0,
            }
    }
    pub(crate) fn local(&self, original: Point) -> Option<Point> {
        let p = self.transform.inverse().map(original);
        let p = if matches!(self.mapping.warp, crate::material_mapping::Warp::None) {
            p
        } else {
            let uv =
                [0, 1].map(|n| (p[n] - self.area.min[n]) / (self.area.max[n] - self.area.min[n]));
            let uv = self.mapping.warp.inverse(uv)?;
            [0, 1].map(|n| self.area.min[n] + uv[n] * (self.area.max[n] - self.area.min[n]))
        };
        self.area.contains(p).then_some(p)
    }
    pub fn pixel(&self, original: Point) -> [u8; 4] {
        let Some(p) = self.local(original) else {
            return [0; 4];
        };
        match &self.source {
            Source::Solid(c) => *c,
            Source::Gradient(s) => s.color(p, None),
            Source::Image(i) => {
                let Some(q) = self
                    .mapping
                    .tiling
                    .pixel(p, [i.width, i.height], self.repeat)
                else {
                    return [0; 4];
                };
                i.pixels
                    .get(q[1] * i.width as usize + q[0])
                    .copied()
                    .unwrap_or([0; 4])
            }
            Source::Metal(m) => m.pixel(p, |q| {
                let i = m.texture.as_ref()?;
                let q = self
                    .mapping
                    .tiling
                    .pixel(q, [i.width, i.height], self.repeat)?;
                i.pixels.get(q[1] * i.width as usize + q[0]).copied()
            }),
        }
    }
}
pub fn load(path: &std::path::Path) -> Result<Source, String> {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let s = crate::image_import::ImageSource::read(path, DEFAULT_DOCUMENT_BUDGET / 4, &cancel)
        .map_err(|e| e.to_string())?;
    let image = s
        .decode(0, true, DEFAULT_DOCUMENT_BUDGET / 4, &cancel)
        .map_err(|e| e.to_string())?
        .image;
    if image.pixels.len() > 4 * 1024 * 1024 {
        return Err("Material source exceeds 4 million pixels".into());
    }
    Ok(Source::Image(Image {
        width: image.width,
        height: image.height,
        pixels: Arc::new(image.pixels),
    }))
}
pub fn prepare(
    d: &Document,
    id: Option<u64>,
    name: &str,
    layer: Layer,
) -> Result<(DocumentState, u64), String> {
    if !layer.valid(false)
        || name.trim().is_empty()
        || name.len() > 1024
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
    {
        return Err("Invalid material, name or snapshot budget".into());
    }
    let mut state = d.snapshot();
    let id = if let Some(id) = id {
        let n = state
            .scene
            .catalog
            .objects
            .index(id, state.scene.objects.len())
            .ok_or("Missing material layer")?;
        if !matches!(state.scene.objects[n], PageObject::Material(_)) {
            return Err("Choose a material layer".into());
        }
        state.scene.objects[n] = PageObject::Material(Box::new(layer));
        id
    } else {
        state
            .scene
            .objects
            .push(PageObject::Material(Box::new(layer)));
        crate::layer_catalog::reconcile_scene(&d.scene, &mut state.scene)?;
        state
            .scene
            .catalog
            .objects
            .id(state.scene.objects.len() - 1)
    };
    state.scene.catalog.object_info.entry(id).or_default().name = Some(name.into());
    if !state.scene.valid()
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&state.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Material scene/Undo budget".into());
    }
    crate::layer_properties::validate_edit(d, &state)?;
    Ok((state, id))
}
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
pub(crate) fn write(scene: &crate::scene::Scene, w: &mut impl Write) -> io::Result<()> {
    for m in scene
        .objects
        .iter()
        .filter_map(|o| {
            if let PageObject::Material(m) = o {
                Some(m.as_ref())
            } else {
                None
            }
        })
        .chain(scene.file_objects.entries.iter().map(|l| &l.cache))
    {
        if let Some(i) = m.source.image() {
            w.write_all(&(i.pixels.len() as u64 * 4).to_le_bytes())?;
            for chunk in i.pixels.chunks(4096) {
                let mut buffer = [0; 16384];
                for (dst, src) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(chunk) {
                    dst.copy_from_slice(src);
                }
                w.write_all(&buffer[..chunk.len() * 4])?;
            }
        }
    }
    Ok(())
}
pub(crate) fn read(scene: &mut crate::scene::Scene, r: &mut impl Read) -> io::Result<()> {
    for m in scene
        .objects
        .iter_mut()
        .filter_map(|o| {
            if let PageObject::Material(m) = o {
                Some(m.as_mut())
            } else {
                None
            }
        })
        .chain(scene.file_objects.entries.iter_mut().map(|l| &mut l.cache))
    {
        if let Some(i) = m.source.image_mut() {
            let mut size = [0; 8];
            r.read_exact(&mut size)?;
            let count = i.width as usize * i.height as usize;
            if u64::from_le_bytes(size) != count as u64 * 4 {
                return Err(invalid("Invalid material image byte count"));
            }
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(count)
                .map_err(|_| invalid("Material allocation failed"))?;
            pixels.resize(count, [0; 4]);
            for chunk in pixels.chunks_mut(4096) {
                let mut buffer = [0; 16384];
                r.read_exact(&mut buffer[..chunk.len() * 4])?;
                for (dst, src) in chunk.iter_mut().zip(buffer.as_chunks::<4>().0.iter()) {
                    dst.copy_from_slice(src);
                }
            }
            i.pixels = Arc::new(pixels);
        }
    }
    Ok(())
}
