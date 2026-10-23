//! Re-editable local mesh recipes. Baked pixels remain ordinary paintable layers.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    mesh_deform::{Mesh, Pin},
    scene::{Point, Rect},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub mesh: Mesh,
    pub puppet: bool,
    pub pins: Vec<Pin>,
    pub selected: Vec<usize>,
}
impl Settings {
    pub fn geometry(&self, area: Rect) -> Result<Mesh, String> {
        let pins = Mesh::puppet(self.mesh.columns, self.mesh.rows, &self.pins, area)?;
        if !self.mesh.valid()
            || !self.mesh.order.is_empty()
            || self.selected.len() > 25
            || self
                .selected
                .iter()
                .enumerate()
                .any(|(n, v)| *v >= self.mesh.points.len() || self.selected[..n].contains(v))
        {
            return Err("Invalid mesh editing state".into());
        }
        if self.puppet {
            Ok(pins)
        } else {
            Ok(self.mesh.clone())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    id: u64,
    index: usize,
    placement: Option<PlacedRaster>,
    precise: bool,
    #[serde(skip)]
    pixels: Vec<[u8; 4]>,
    #[serde(skip)]
    high: Vec<[u16; 4]>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub settings: Settings,
    pub area: Rect,
    pub selection: bool,
    width: u32,
    height: u32,
    frame: u32,
    original_selection: Option<Rect>,
    original_mask: Option<crate::selection::Mask>,
    original_translation: Point,
    sources: Vec<Source>,
    stamp: u64,
}
impl Edit {
    pub fn layers(&self) -> Vec<usize> {
        self.sources.iter().map(|s| s.index).collect()
    }
    pub fn bytes(&self) -> usize {
        self.sources
            .iter()
            .map(|s| {
                let e = s.placement.unwrap_or(PlacedRaster {
                    width: self.width,
                    height: self.height,
                    offset: [0; 2],
                });
                (e.width as usize)
                    .saturating_mul(e.height as usize)
                    .saturating_mul(if s.precise { 12 } else { 4 })
            })
            .fold(0usize, usize::saturating_add)
            .saturating_add(
                self.original_mask
                    .as_ref()
                    .map_or(0, |m| m.runs.len().saturating_mul(32)),
            )
            .saturating_add(4096)
    }
    pub fn valid(&self, missing: bool) -> bool {
        (1..=32768).contains(&self.width)
            && (1..=32768).contains(&self.height)
            && self.area.valid()
            && (0..2).all(|k| self.area.max[k] > self.area.min[k])
            && self.original_selection.is_none_or(Rect::valid)
            && self.original_mask.as_ref().is_none_or(|m| m.valid())
            && self
                .original_translation
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
            && !self.sources.is_empty()
            && self.sources.len() <= 32
            && self.sources.iter().enumerate().all(|(n, s)| {
                let e = s.placement.unwrap_or(PlacedRaster {
                    width: self.width,
                    height: self.height,
                    offset: [0; 2],
                });
                let count = e.width as usize * e.height as usize;
                s.id > 0
                    && s.index < 256
                    && e.valid()
                    && count <= 4 * 1024 * 1024
                    && !self.sources[..n]
                        .iter()
                        .any(|p| p.id == s.id || p.index == s.index)
                    && (missing && s.pixels.is_empty() && s.high.is_empty()
                        || s.pixels.len() == count
                            && if s.precise {
                                s.high.len() == count
                                    && s.high.iter().zip(&s.pixels).all(|(h, l)| {
                                        crate::color_precision::Sample::from16(*h).rgba8() == *l
                                    })
                            } else {
                                s.high.is_empty()
                            })
            })
            && self.bytes() <= 32 * 1024 * 1024
            && self.settings.geometry(self.area).is_ok_and(|m| m.valid())
    }
    pub fn matches(&self, d: &Document) -> bool {
        self.valid(false)
            && self.width == d.width
            && self.height == d.height
            && self.frame == d.current_frame
            && self.stamp == stamp(d, &self.sources, self.selection)
            && self.sources.iter().all(|s| {
                s.index < d.layers.len()
                    && crate::layer_properties::raster(d, s.index)
                        == crate::layer_properties::Target::Raster(s.id)
            })
    }
    pub fn original(&self, d: &Document) -> Result<Document, String> {
        if !self.matches(d) {
            return Err("Saved mesh source no longer matches this page/frame/layer/selection. Start a new mesh edit instead.".into());
        }
        let mut state = d.snapshot();
        state.scene.mesh_edit = None;
        for s in &self.sources {
            let l = &mut state.layers[s.index];
            l.placement = s.placement;
            l.pixels = s.pixels.clone();
            l.precise = s.precise.then(|| s.high.clone());
        }
        if self.selection {
            state.scene.selection = self.original_selection;
            state.scene.selection_mask = self.original_mask.clone();
            state.scene.selection_translation = self.original_translation;
        }
        if !state.color.valid_layers(&state.layers) {
            return Err("Saved mesh source color precision mismatch".into());
        }
        crate::layer_merge::preview(&state, d.current_frame)
    }
}
fn stamp(d: &Document, sources: &[Source], selection: bool) -> u64 {
    // Deterministic stale-edit fingerprint, not an authenticity/security hash.
    let mut h = 0xcbf29ce484222325u64;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
        }
    };
    feed(&d.width.to_le_bytes());
    feed(&d.height.to_le_bytes());
    feed(&d.current_frame.to_le_bytes());
    feed(&serde_json::to_vec(&d.color).unwrap_or_default());
    feed(&serde_json::to_vec(&d.scene.transform).unwrap_or_default());
    if selection {
        feed(
            &serde_json::to_vec(&(
                d.scene.selection,
                &d.scene.selection_mask,
                d.scene.selection_translation,
            ))
            .unwrap_or_default(),
        );
    }
    for s in sources {
        if let Some(l) = d.layers.get(s.index) {
            feed(&serde_json::to_vec(&l.placement).unwrap_or_default());
            for p in &l.pixels {
                feed(p);
            }
            if let Some(pixels) = &l.precise {
                for p in pixels {
                    for v in p {
                        feed(&v.to_le_bytes());
                    }
                }
            }
        } else {
            feed(b"missing");
        }
    }
    h
}
pub fn prepare(
    d: &Document,
    layers: &[usize],
    selection: bool,
    settings: &Settings,
    resume: bool,
    tick: &mut impl FnMut() -> Result<(), String>,
) -> Result<DocumentState, String> {
    tick()?;
    let previous = if resume {
        Some(d.scene.mesh_edit.as_ref().ok_or("No saved mesh edit")?)
    } else {
        None
    };
    let source = previous.map(|e| e.original(d)).transpose()?;
    let input = source.as_ref().unwrap_or(d);
    let layers = crate::deform::targets(input, layers)?;
    if let Some(e) = previous {
        if e.layers() != layers || e.selection != selection {
            return Err("Resume keeps the saved source layers and pixel selection".into());
        }
        if e.settings == *settings {
            return Ok(d.snapshot());
        }
    }
    let area = crate::deform::area(input, &layers, selection)?.unwrap_or(Rect {
        min: [0.; 2],
        max: [input.width as f64, input.height as f64],
    });
    let mesh = settings.geometry(area)?;
    let mut out = crate::deform::prepare_with_progress(
        input,
        &layers,
        selection,
        crate::deform::Change::Mesh(&mesh),
        tick,
    )?;
    if out == input.snapshot() && !resume && settings.pins.is_empty() {
        return Ok(d.snapshot());
    }
    let sources = layers
        .iter()
        .map(|n| {
            let l = &input.layers[*n];
            let crate::layer_properties::Target::Raster(id) =
                crate::layer_properties::raster(input, *n)
            else {
                unreachable!()
            };
            Source {
                id,
                index: *n,
                placement: l.placement,
                precise: l.precise.is_some(),
                pixels: l.pixels.clone(),
                high: l.precise.clone().unwrap_or_default(),
            }
        })
        .collect();
    let mut edit = Edit {
        settings: settings.clone(),
        area,
        selection,
        width: d.width,
        height: d.height,
        frame: d.current_frame,
        original_selection: input.scene.selection,
        original_mask: input.scene.selection_mask.clone(),
        original_translation: input.scene.selection_translation,
        sources,
        stamp: 0,
    };
    let rendered = crate::layer_merge::preview(&out, d.current_frame)?;
    edit.stamp = stamp(&rendered, &edit.sources, selection);
    if !edit.valid(false) {
        return Err("Editable mesh source exceeds32MiB or contains invalid controls".into());
    }
    out.scene.mesh_edit = Some(Box::new(edit));
    if out.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4 {
        return Err("Editable mesh source/output exceeds Undo memory budget".into());
    }
    Ok(out)
}
fn invalid(e: impl ToString) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
}
pub(crate) fn write(scene: &crate::scene::Scene, w: &mut impl Write) -> std::io::Result<()> {
    if let Some(e) = &scene.mesh_edit {
        for s in &e.sources {
            for p in &s.pixels {
                w.write_all(p)?;
            }
            for p in &s.high {
                for v in p {
                    w.write_all(&v.to_le_bytes())?;
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn read(scene: &mut crate::scene::Scene, r: &mut impl Read) -> std::io::Result<()> {
    let Some(e) = &mut scene.mesh_edit else {
        return Ok(());
    };
    if !e.valid(true) {
        return Err(invalid("Invalid mesh source metadata"));
    }
    for s in &mut e.sources {
        let extent = s.placement.unwrap_or(PlacedRaster {
            width: e.width,
            height: e.height,
            offset: [0; 2],
        });
        let n = extent.width as usize * extent.height as usize;
        s.pixels.try_reserve_exact(n).map_err(invalid)?;
        for _ in 0..n {
            let mut p = [0; 4];
            r.read_exact(&mut p)?;
            s.pixels.push(p);
        }
        if s.precise {
            s.high.try_reserve_exact(n).map_err(invalid)?;
            for _ in 0..n {
                let mut p = [0; 4];
                for v in &mut p {
                    let mut b = [0; 2];
                    r.read_exact(&mut b)?;
                    *v = u16::from_le_bytes(b);
                }
                s.high.push(p);
            }
        }
    }
    if !e.valid(false) {
        return Err(invalid("Invalid mesh source pixel payload"));
    }
    Ok(())
}
