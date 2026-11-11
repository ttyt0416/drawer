//! Sparse64×64 RGBA tiles. Current pixels are never evicted by Undo-cache pruning.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub const SIDE: u32 = 64;
pub const PIXELS: usize = SIDE as usize * SIDE as usize;
type Pixel = [u8; 4];
#[derive(Clone, Debug)]
pub struct Tile {
    data: Arc<Vec<Pixel>>,
    start: usize,
}
impl Tile {
    fn slice(&self) -> &[Pixel] {
        &self.data[self.start..self.start + PIXELS]
    }
    fn make_mut(&mut self) -> Result<&mut [Pixel], String> {
        if self.start != 0 || self.data.len() != PIXELS || Arc::strong_count(&self.data) > 1 {
            let mut data = Vec::new();
            data.try_reserve_exact(PIXELS)
                .map_err(|_| "Not enough memory for a raster tile")?;
            data.extend_from_slice(self.slice());
            self.data = Arc::new(data);
            self.start = 0;
        }
        Ok(Arc::get_mut(&mut self.data)
            .ok_or("Raster tile still shared")?
            .as_mut_slice())
    }
}
impl PartialEq for Tile {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.data, &other.data) && self.start == other.start)
            || self.slice() == other.slice()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub default: Pixel,
    tiles: BTreeMap<[u32; 2], Tile>,
}
impl Image {
    pub fn new(width: u32, height: u32, default: Pixel) -> Self {
        Self {
            width,
            height,
            default,
            tiles: BTreeMap::new(),
        }
    }
    fn address(&self, n: usize) -> Option<([u32; 2], usize)> {
        if n >= self.width as usize * self.height as usize {
            return None;
        }
        let x = n % self.width as usize;
        let y = n / self.width as usize;
        Some((
            [x as u32 / SIDE, y as u32 / SIDE],
            (y % SIDE as usize) * SIDE as usize + x % SIDE as usize,
        ))
    }
    pub fn get(&self, n: usize) -> Option<Pixel> {
        let (key, i) = self.address(n)?;
        Some(self.tiles.get(&key).map_or(self.default, |t| t.slice()[i]))
    }
    pub fn bytes(&self) -> usize {
        let mut seen = BTreeSet::new();
        self.tiles
            .values()
            .filter(|t| seen.insert(Arc::as_ptr(&t.data) as usize))
            .map(|t| t.data.capacity() * 4)
            .sum::<usize>()
            + self.tiles.len() * 96
            + std::mem::size_of::<Self>()
    }
    pub fn valid(&self) -> bool {
        (1..=crate::document::MAX_DIMENSION).contains(&self.width)
            && (1..=crate::document::MAX_DIMENSION).contains(&self.height)
            && self.tiles.iter().all(|(p, t)| {
                p[0] * SIDE < self.width
                    && p[1] * SIDE < self.height
                    && t.start
                        .checked_add(PIXELS)
                        .is_some_and(|n| n <= t.data.len())
            })
    }
    /// Prepare every touched tile on a detached candidate before publishing pixels.
    pub fn prepare(
        &mut self,
        offsets: impl Iterator<Item = usize>,
        budget: usize,
    ) -> Result<(), String> {
        let mut keys = BTreeSet::new();
        for n in offsets {
            keys.insert(self.address(n).ok_or("Raster tile offset out of range")?.0);
        }
        let extra = keys
            .iter()
            .filter(|p| {
                self.tiles.get(*p).is_none_or(|t| {
                    t.start != 0 || t.data.len() != PIXELS || Arc::strong_count(&t.data) > 1
                })
            })
            .count()
            .checked_mul(PIXELS * 4 + 96)
            .ok_or("Raster tile memory overflow")?
            .saturating_add(self.tiles.len() * 96);
        if self.bytes().saturating_add(extra) > budget {
            return Err(format!(
                "Raster tile allocation needs {} bytes; available budget {budget}",
                self.bytes().saturating_add(extra)
            ));
        }
        for key in keys {
            if let Some(tile) = self.tiles.get_mut(&key) {
                tile.make_mut()?;
            } else {
                let mut data = Vec::new();
                data.try_reserve_exact(PIXELS)
                    .map_err(|_| "Not enough memory for a raster tile")?;
                data.resize(PIXELS, self.default);
                self.tiles.insert(
                    key,
                    Tile {
                        data: Arc::new(data),
                        start: 0,
                    },
                );
            }
        }
        Ok(())
    }
    pub fn set_prepared(&mut self, n: usize, pixel: Pixel) {
        let (key, i) = self.address(n).expect("Prepared tile offset");
        let tile = self.tiles.get_mut(&key).expect("Prepared tile");
        Arc::get_mut(&mut tile.data).expect("Unique prepared tile")[i] = pixel;
    }
    pub fn is_transparent(&self) -> bool {
        if self.default[3] != 0
            && self.tiles.len() < (self.width.div_ceil(SIDE) * self.height.div_ceil(SIDE)) as usize
        {
            return false;
        }
        self.tiles.iter().all(|(p, t)| {
            t.slice().iter().enumerate().all(|(n, v)| {
                p[0] * SIDE + (n % SIDE as usize) as u32 >= self.width
                    || p[1] * SIDE + (n / SIDE as usize) as u32 >= self.height
                    || v[3] == 0
            })
        })
    }
    pub fn compact(&mut self) {
        let default = self.default;
        self.tiles
            .retain(|_, t| t.slice().iter().any(|p| *p != default));
    }
    pub(crate) fn stored(&self) -> Vec<([u32; 2], &[Pixel])> {
        self.tiles
            .iter()
            .filter(|(_, t)| t.slice().iter().any(|p| *p != self.default))
            .map(|(p, t)| (*p, t.slice()))
            .collect()
    }
}
pub(crate) fn history_bytes(
    before: &crate::sizing::DocumentState,
    after: &crate::sizing::DocumentState,
) -> usize {
    if before
        .layers
        .iter()
        .chain(&after.layers)
        .all(|l| l.tiled.is_none())
    {
        return before.memory_bytes().saturating_add(after.memory_bytes());
    }
    let mut a = BTreeMap::new();
    let mut b = BTreeMap::new();
    for (state, set) in [(before, &mut a), (after, &mut b)] {
        for image in state.layers.iter().filter_map(|l| l.tiled.as_ref()) {
            for tile in image.tiles.values() {
                set.insert(Arc::as_ptr(&tile.data) as usize, tile.data.capacity() * 4);
            }
        }
    }
    let shared = a
        .iter()
        .filter(|(p, _)| b.contains_key(p))
        .map(|(_, n)| *n)
        .sum::<usize>();
    before
        .allocation_bytes()
        .saturating_add(after.allocation_bytes())
        .saturating_sub(shared.saturating_mul(2))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stored {
    pub layer: usize,
    pub default: Pixel,
    pub coordinates: Vec<[u32; 2]>,
}
pub(crate) fn metadata(d: &crate::document::Document) -> Option<Vec<Stored>> {
    let entries = d
        .layers
        .iter()
        .enumerate()
        .filter_map(|(layer, l)| {
            l.tiled.as_ref().map(|t| Stored {
                layer,
                default: t.default,
                coordinates: t.stored().into_iter().map(|(p, _)| p).collect(),
            })
        })
        .collect::<Vec<_>>();
    (!entries.is_empty()).then_some(entries)
}
pub(crate) fn allocated_layers(layers: &[crate::document::RasterLayer]) -> usize {
    let mut pointers = BTreeMap::new();
    let mut total = 0usize;
    for layer in layers {
        if let Some(image) = &layer.tiled {
            total = total.saturating_add(image.tiles.len() * 96 + std::mem::size_of::<Image>());
            for tile in image.tiles.values() {
                pointers.insert(Arc::as_ptr(&tile.data) as usize, tile.data.capacity() * 4);
            }
        } else {
            total = total.saturating_add(layer.allocation_bytes());
        }
    }
    total.saturating_add(pointers.values().sum::<usize>())
}
pub(crate) fn clone_layers_bytes(layers: &[crate::document::RasterLayer]) -> usize {
    layers
        .iter()
        .map(|l| {
            l.tiled.as_ref().map_or_else(
                || l.allocation_bytes(),
                |t| t.tiles.len() * 96 + std::mem::size_of::<Image>(),
            )
        })
        .sum()
}
pub(crate) fn change_bytes(
    d: &crate::document::Document,
    next: &crate::sizing::DocumentState,
) -> usize {
    if d.layers
        .iter()
        .chain(&next.layers)
        .all(|l| l.tiled.is_none())
    {
        return d.memory_bytes().saturating_add(next.memory_bytes());
    }
    let mut a = BTreeMap::new();
    let mut b = BTreeSet::new();
    for image in d.layers.iter().filter_map(|l| l.tiled.as_ref()) {
        for t in image.tiles.values() {
            a.insert(Arc::as_ptr(&t.data) as usize, t.data.capacity() * 4);
        }
    }
    for image in next.layers.iter().filter_map(|l| l.tiled.as_ref()) {
        for t in image.tiles.values() {
            b.insert(Arc::as_ptr(&t.data) as usize);
        }
    }
    let shared = a
        .iter()
        .filter(|(p, _)| b.contains(p))
        .map(|(_, n)| *n)
        .sum::<usize>();
    d.allocation_bytes()
        .saturating_add(next.allocation_bytes())
        .saturating_sub(shared.saturating_mul(2))
}
pub(crate) fn hydrate(
    d: &mut crate::document::Document,
    stored: Vec<Stored>,
    budget: usize,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for s in stored {
        if !seen.insert(s.layer) {
            return Err("Duplicate sparse raster layer".into());
        }
        let layer = d
            .layers
            .get_mut(s.layer)
            .ok_or("Missing sparse raster layer")?;
        let count = s.coordinates.len();
        let packed = layer.placement.ok_or("Sparse native packing missing")?;
        if count > 32768
            || layer.precise.is_some()
            || if count == 0 {
                packed.width != 1 || packed.height != 1 || layer.pixels.len() != 1
            } else {
                packed.width != PIXELS as u32
                    || packed.height != count as u32
                    || layer.pixels.len() != count * PIXELS
            }
        {
            return Err("Invalid sparse native packing".into());
        }
        let data = Arc::new(std::mem::take(&mut layer.pixels));
        let mut image = Image::new(d.width, d.height, s.default);
        for (n, p) in s.coordinates.into_iter().enumerate() {
            if p[0] >= d.width.div_ceil(SIDE)
                || p[1] >= d.height.div_ceil(SIDE)
                || image
                    .tiles
                    .insert(
                        p,
                        Tile {
                            data: data.clone(),
                            start: n * PIXELS,
                        },
                    )
                    .is_some()
            {
                return Err("Invalid/duplicate raster tile coordinate".into());
            }
        }
        layer.placement = None;
        layer.tiled = Some(image);
    }
    if d.allocation_bytes() > budget {
        return Err("Sparse document allocation exceeds memory budget".into());
    }
    Ok(())
}

#[path = "tiled_raster_verify.rs"]
pub mod verify;
