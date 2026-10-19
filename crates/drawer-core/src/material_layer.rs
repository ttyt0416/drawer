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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Source {
    Solid([u8; 4]),
    Gradient(Box<gradient::Settings>),
    Image(Image),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub source: Source,
    pub area: Rect,
    /// Local coordinates to preserved original-page coordinates.
    pub transform: Affine,
    pub repeat: [bool; 2],
}
impl Layer {
    pub fn valid(&self, missing: bool) -> bool {
        self.transform.valid()
            && self.area.valid()
            && [self.area.min, self.area.max]
                .into_iter()
                .flatten()
                .all(|v| v.abs() <= 1e9)
            && self.bounds().valid()
            && match &self.source {
                Source::Solid(_) => true,
                Source::Gradient(s) => s.valid() && s.shape != gradient::Shape::Freeform,
                Source::Image(i) => {
                    (1..=32768).contains(&i.width)
                        && (1..=32768).contains(&i.height)
                        && u64::from(i.width) * u64::from(i.height) <= 4 * 1024 * 1024
                        && (i.pixels.len() == i.width as usize * i.height as usize
                            || (missing && i.pixels.is_empty()))
                }
            }
    }
    pub fn bounds(&self) -> Rect {
        let points = [
            self.area.min,
            [self.area.max[0], self.area.min[1]],
            self.area.max,
            [self.area.min[0], self.area.max[1]],
        ]
        .map(|p| self.transform.map(p));
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
        match &self.source {
            Source::Image(i) => i.width as usize * i.height as usize * 4,
            Source::Gradient(s) => serde_json::to_vec(s).map_or(0, |b| b.len()),
            _ => 0,
        }
    }
    pub fn pixel(&self, original: Point) -> [u8; 4] {
        let p = self.transform.inverse().map(original);
        if !self.area.contains(p) {
            return [0; 4];
        }
        match &self.source {
            Source::Solid(c) => *c,
            Source::Gradient(s) => s.color(p, None),
            Source::Image(i) => {
                let mut q = [p[0].floor(), p[1].floor()];
                for (n, size) in [i.width, i.height].into_iter().enumerate() {
                    if self.repeat[n] {
                        q[n] = q[n].rem_euclid(f64::from(size));
                    }
                    if q[n] < 0. || q[n] >= f64::from(size) {
                        return [0; 4];
                    }
                }
                i.pixels
                    .get(q[1] as usize * i.width as usize + q[0] as usize)
                    .copied()
                    .unwrap_or([0; 4])
            }
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
    for o in &scene.objects {
        if let PageObject::Material(m) = o
            && let Source::Image(i) = &m.source
        {
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
    for o in &mut scene.objects {
        if let PageObject::Material(m) = o
            && let Source::Image(i) = &mut m.source
        {
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
