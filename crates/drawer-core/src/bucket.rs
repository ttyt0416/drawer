//! Owned four-connected fill. Alpha threshold, color distance, gap closure and
//! edge coverage are separate controls; discovery never changes the document.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster},
    scene::Point,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Strict,
    CloseGaps,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reference {
    #[default]
    Active,
    Composite,
    Layer(u16),
    Marked,
    Selected,
    Folder(u16),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expansion {
    #[default]
    Diamond,
    Rectangle,
    Round,
    Darkest,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub advanced: crate::extended_fill::Settings,
    #[serde(default)]
    pub exclude: crate::fill_reference::Filter,
    #[serde(default)]
    pub selected: [u64; 4],
    #[serde(default)]
    pub selected_vectors: [u64; 4],
    #[serde(default)]
    pub expansion: Expansion,
    #[serde(default)]
    pub fill_narrow: bool,
    pub mode: Mode,
    pub reference: Reference,
    pub color_tolerance: f64,
    pub alpha_tolerance: f64,
    pub gap: u8,
    pub expand: u8,
    pub antialias: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            include_hidden: false,
            advanced: Default::default(),
            exclude: Default::default(),
            selected: [0; 4],
            selected_vectors: [0; 4],
            expansion: Expansion::Diamond,
            fill_narrow: false,
            mode: Mode::Strict,
            reference: Reference::Active,
            color_tolerance: 0.0,
            alpha_tolerance: 0.0,
            gap: 1,
            expand: 0,
            antialias: true,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.advanced.valid()
            && [self.color_tolerance, self.alpha_tolerance]
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.gap <= 32
            && self.expand <= 32
            && !matches!(self.reference,Reference::Layer(n)|Reference::Folder(n) if n>=256)
    }
}
pub struct Region {
    pub width: u32,
    pub height: u32,
    coverage: Vec<u8>,
}
impl Region {
    pub fn coverage(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            0
        } else {
            self.coverage[y as usize * self.width as usize + x as usize]
        }
    }
    pub fn build(doc: &Document, active: usize, seed: Point, s: Settings) -> Result<Self, String> {
        Self::build_with_connectivity(doc, active, seed, s, true)
    }
    pub fn build_with_connectivity(
        doc: &Document,
        active: usize,
        seed: Point,
        s: Settings,
        connected: bool,
    ) -> Result<Self, String> {
        if !s.valid()
            || seed.iter().any(|v| !v.is_finite())
            || seed[0] < 0.0
            || seed[1] < 0.0
            || seed[0] >= doc.width as f64
            || seed[1] >= doc.height as f64
        {
            return Err("Invalid fill settings or seed".into());
        }
        let count = doc.width as usize * doc.height as usize;
        if count > 4 * 1024 * 1024
            || count
                .saturating_mul(24)
                .saturating_add(doc.resident_bytes())
                > DEFAULT_DOCUMENT_BUDGET
        {
            return Err("Fill region exceeds the work/memory limit".into());
        }
        let frame = crate::extended_fill::frame(doc, s.advanced)?;
        let connected = connected && s.advanced.connected;
        let sampler = crate::fill_reference::Sampler::new(doc, active, s)?;
        let sample = |x, y| sampler.pixel(x, y);
        let start = seed[1].floor() as usize * doc.width as usize + seed[0].floor() as usize;
        let origin = sample(seed[0].floor() as u32, seed[1].floor() as u32);
        let transparent = s.advanced.transparent(origin, s.alpha_tolerance);
        let mut eligible = vec![0u8; count];
        let mut alpha = vec![0u8; count];
        let mut domain = vec![0u8; count];
        let mut writable = vec![0u8; count];
        let mut brightness = vec![255u8; count];
        for y in 0..doc.height {
            for x in 0..doc.width {
                let i = y as usize * doc.width as usize + x as usize;
                let p = sample(x, y);
                alpha[i] = p[3];
                let rgb = (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19 + 128) / 256;
                brightness[i] = (255 - ((255 - rgb) * p[3] as u32 + 127) / 255) as u8;
                domain[i] = u8::from(crate::extended_fill::inside(
                    doc,
                    frame,
                    [x as f64 + 0.5, y as f64 + 0.5],
                ));
                writable[i] = u8::from(s.advanced.writable(p, s.alpha_tolerance));
                eligible[i] = u8::from(domain[i] != 0 && s.advanced.eligible(p, origin, s));
            }
        }
        let mut closed = eligible.clone();
        if s.mode == Mode::CloseGaps && s.gap > 0 {
            for (lines, length, stride) in [
                (doc.height as usize, doc.width as usize, 1usize),
                (doc.width as usize, doc.height as usize, doc.width as usize),
            ] {
                for line in 0..lines {
                    let offset = if stride == 1 {
                        line * doc.width as usize
                    } else {
                        line
                    };
                    let mut i = 0;
                    while i < length {
                        if eligible[offset + i * stride] == 0 {
                            i += 1;
                            continue;
                        }
                        let a = i;
                        while i < length && eligible[offset + i * stride] != 0 {
                            i += 1;
                        }
                        if a > 0 && i < length && i - a <= s.gap as usize {
                            for j in a..i {
                                closed[offset + j * stride] = 0;
                            }
                        }
                    }
                }
            }
        }
        if domain[start] == 0 {
            return Err("Fill seed is outside the active output frame".into());
        }
        let mut queue = if s.advanced.target.enclosed() {
            crate::extended_fill::enclosed(
                &mut closed,
                &domain,
                doc.width as usize,
                doc.height as usize,
                start,
                connected,
            )?
        } else {
            if connected && closed[start] == 0 {
                return Err("Seed is outside the target color domain or on a closed gap".into());
            }
            let mut queue = Vec::<u32>::new();
            queue
                .try_reserve_exact(count)
                .map_err(|_| "Fill queue allocation failed")?;
            if connected {
                queue.push(start as u32);
                closed[start] = 2;
                let mut head = 0;
                while head < queue.len() {
                    let p = queue[head] as usize;
                    head += 1;
                    for q in neighbors(p, doc.width as usize, doc.height as usize)
                        .into_iter()
                        .flatten()
                    {
                        if closed[q] == 1 {
                            closed[q] = 2;
                            queue.push(q as u32);
                        }
                    }
                }
            } else {
                for (i, value) in closed.iter_mut().enumerate() {
                    if *value != 0 {
                        *value = 2;
                        queue.push(i as u32);
                    }
                }
            }
            queue
        };
        let mut edge_settings = s;
        if s.advanced.include_outlines {
            edge_settings.expand = edge_settings.expand.max(s.advanced.outline_distance);
        }
        let distance = expand_region(
            &mut queue,
            &eligible,
            &brightness,
            doc.width as usize,
            doc.height as usize,
            edge_settings,
        )?;
        let mut coverage = vec![0u8; count];
        for &p in &queue {
            let i = p as usize;
            coverage[i] = 255;
        }
        if transparent && s.antialias {
            for &p in &queue {
                for q in neighbors(p as usize, doc.width as usize, doc.height as usize)
                    .into_iter()
                    .flatten()
                {
                    if distance[q] == 255 && alpha[q] > 0 && alpha[q] < 255 {
                        coverage[q] = coverage[q].max(255 - alpha[q]);
                    }
                }
            }
        }
        for (i, c) in coverage.iter_mut().enumerate() {
            if domain[i] == 0 || writable[i] == 0 {
                *c = 0;
            }
        }
        Ok(Self {
            width: doc.width,
            height: doc.height,
            coverage,
        })
    }
}
fn expand_region(
    queue: &mut Vec<u32>,
    eligible: &[u8],
    brightness: &[u8],
    w: usize,
    h: usize,
    s: Settings,
) -> Result<Vec<u8>, String> {
    let count = w * h;
    let mut narrow = vec![255u8; count];
    for &p in queue.iter() {
        narrow[p as usize] = 0;
    }
    let mut head = 0;
    let mut work = 0usize;
    let limit = s.gap.max(1);
    while s.fill_narrow && head < queue.len() {
        let p = queue[head] as usize;
        head += 1;
        if narrow[p] >= limit {
            continue;
        }
        for q in neighbors(p, w, h).into_iter().flatten() {
            if narrow[q] != 255 || eligible[q] == 0 {
                continue;
            }
            let mut thin = false;
            for (stride, at, length) in [(1, q % w, w), (w, q / w, h)] {
                let mut span = 1usize;
                for direction in [-1i64, 1] {
                    for n in 1..=limit as usize {
                        work += 1;
                        if work > 64 * 1024 * 1024 {
                            return Err("Narrow-area sampling work limit".into());
                        }
                        let v = at as i64 + direction * n as i64;
                        if v < 0 || v >= length as i64 {
                            break;
                        }
                        let index = (q as i64 + direction * n as i64 * stride as i64) as usize;
                        if eligible[index] == 0 {
                            break;
                        }
                        span += 1;
                    }
                }
                thin |= span <= limit as usize;
            }
            if thin {
                narrow[q] = narrow[p] + 1;
                queue.push(q as u32);
            }
        }
    }
    let mut distance = vec![255u8; count];
    for &p in queue.iter() {
        distance[p as usize] = 0;
    }
    if s.expand == 0 {
        return Ok(distance);
    }
    if s.expansion == Expansion::Round {
        let r = s.expand as i32;
        let mut offsets = vec![];
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r * r {
                    offsets.push((x, y));
                }
            }
        }
        let base = distance.clone();
        if (count - queue.len()).saturating_mul(offsets.len()) > 64 * 1024 * 1024 {
            return Err("Round expansion work limit".into());
        }
        for p in 0..count {
            if base[p] == 0 {
                continue;
            }
            let (x, y) = ((p % w) as i32, (p / w) as i32);
            if offsets.iter().any(|(dx, dy)| {
                let a = x + dx;
                let b = y + dy;
                a >= 0
                    && b >= 0
                    && a < w as i32
                    && b < h as i32
                    && base[b as usize * w + a as usize] == 0
            }) {
                distance[p] = 1;
                queue.push(p as u32);
            }
        }
        return Ok(distance);
    }
    head = 0;
    while head < queue.len() {
        let p = queue[head] as usize;
        head += 1;
        if distance[p] >= s.expand {
            continue;
        }
        let mut candidates = [None; 8];
        candidates[..4].copy_from_slice(&neighbors(p, w, h));
        if s.expansion == Expansion::Rectangle {
            let x = p % w;
            let y = p / w;
            for dy in [-1i64, 1] {
                for dx in [-1i64, 1] {
                    let a = x as i64 + dx;
                    let b = y as i64 + dy;
                    if a >= 0 && b >= 0 && a < w as i64 && b < h as i64 {
                        let slot = 4 + usize::from(dy > 0) * 2 + usize::from(dx > 0);
                        candidates[slot] = Some(b as usize * w + a as usize);
                    }
                }
            }
        }
        for q in candidates.into_iter().flatten() {
            if distance[q] == 255
                && (s.expansion != Expansion::Darkest || brightness[q] <= brightness[p])
            {
                distance[q] = distance[p] + 1;
                queue.push(q as u32);
            }
        }
    }
    Ok(distance)
}
pub(crate) fn neighbors(p: usize, w: usize, h: usize) -> [Option<usize>; 4] {
    let x = p % w;
    let y = p / w;
    [
        (x > 0).then(|| p - 1),
        (x + 1 < w).then_some(p + 1),
        (y > 0).then(|| p - w),
        (y + 1 < h).then_some(p + w),
    ]
}
/// None fills the entire selection directly, bypassing seed/color/gap discovery.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    doc: &Document,
    layer: usize,
    seed: Option<Point>,
    color: [u8; 4],
    erase: bool,
    alpha_lock: bool,
    settings: Settings,
) -> Result<DocumentState, String> {
    let raster = doc.layers.get(layer).ok_or("Select a raster fill target")?;
    if raster.locked
        || !doc.scene.paper.raster_visible(layer, doc.layers.len())
        || !raster.valid(doc.width, doc.height)
        || !doc.scene.valid()
    {
        return Err("Fill target is hidden, locked or invalid".into());
    }
    if !settings.valid() {
        return Err("Invalid fill settings".into());
    }
    if doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Fill snapshot exceeds undo budget".into());
    }
    let region = seed
        .map(|p| Region::build(doc, layer, p, settings))
        .transpose()?;
    let coverage = |x: u32, y: u32| -> u8 {
        let mask = doc
            .scene
            .selection_coverage([x as f64 + 0.5, y as f64 + 0.5]) as u32;
        ((mask * region.as_ref().map_or(255, |r| r.coverage(x, y)) as u32 + 127) / 255) as u8
    };
    paint_coverage(doc, layer, color, erase, alpha_lock, coverage)
}
pub(crate) fn paint_coverage(
    doc: &Document,
    layer: usize,
    color: [u8; 4],
    erase: bool,
    alpha_lock: bool,
    coverage: impl Fn(u32, u32) -> u8,
) -> Result<DocumentState, String> {
    paint(doc, layer, erase, alpha_lock, |x, y| {
        (color, coverage(x, y))
    })
}
pub(crate) fn paint_pixels(
    doc: &Document,
    layer: usize,
    alpha_lock: bool,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) -> Result<DocumentState, String> {
    paint(doc, layer, false, alpha_lock, |x, y| {
        let c = pixel(x, y);
        (c, if c[3] > 0 { 255 } else { 0 })
    })
}
fn paint(
    doc: &Document,
    layer: usize,
    erase: bool,
    alpha_lock: bool,
    sample: impl Fn(u32, u32) -> ([u8; 4], u8),
) -> Result<DocumentState, String> {
    crate::layer_properties::paintable(doc, layer)?;
    let alpha_lock = alpha_lock || crate::clipping::alpha_locked(doc, layer);
    let raster = doc
        .layers
        .get(layer)
        .ok_or("Select a raster paint target")?;
    if raster.locked
        || !doc.scene.paper.raster_visible(layer, doc.layers.len())
        || !raster.valid(doc.width, doc.height)
        || !doc.scene.valid()
    {
        return Err("Paint target is hidden, locked or invalid".into());
    }
    if doc.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Paint snapshot exceeds undo budget".into());
    }
    let mut min = [doc.width as i64, doc.height as i64];
    let mut max = [0i64; 2];
    let count = doc.width as usize * doc.height as usize;
    if count > 4 * 1024 * 1024 {
        return Err("Fill exceeds canvas work limit".into());
    }
    for y in 0..doc.height {
        for x in 0..doc.width {
            if sample(x, y).1 > 0
                && (!alpha_lock || raster.pixel(doc.width, doc.height, x as i64, y as i64)[3] > 0)
            {
                min = [min[0].min(x as i64), min[1].min(y as i64)];
                max = [max[0].max(x as i64 + 1), max[1].max(y as i64 + 1)];
            }
        }
    }
    if min[0] >= max[0] || min[1] >= max[1] || (alpha_lock && erase) {
        return Ok(doc.snapshot());
    }
    let old = raster.extent(doc.width, doc.height);
    let offset = [old.offset[0].min(min[0]), old.offset[1].min(min[1])];
    let end = [
        (old.offset[0] + old.width as i64).max(max[0]),
        (old.offset[1] + old.height as i64).max(max[1]),
    ];
    let extent = PlacedRaster {
        width: (end[0] - offset[0]) as u32,
        height: (end[1] - offset[1]) as u32,
        offset,
    };
    let bytes = extent.width as usize * extent.height as usize * 4;
    if !extent.valid()
        || bytes.saturating_add(doc.memory_bytes()).saturating_mul(2) > DEFAULT_DOCUMENT_BUDGET / 4
        || bytes.saturating_mul(3).saturating_add(doc.resident_bytes()) > DEFAULT_DOCUMENT_BUDGET
    {
        return Err("Expanded fill raster exceeds memory/undo limits".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(bytes / 4)
        .map_err(|_| "Fill allocation failed")?;
    pixels.resize(bytes / 4, [0; 4]);
    let index = |x: i64, y: i64| ((y - offset[1]) * extent.width as i64 + x - offset[0]) as usize;
    for y in 0..old.height as i64 {
        for x in 0..old.width as i64 {
            pixels[index(old.offset[0] + x, old.offset[1] + y)] =
                raster.pixels[(y * old.width as i64 + x) as usize];
        }
    }
    let mut changed = false;
    for y in min[1]..max[1] {
        for x in min[0]..max[0] {
            let (color, c) = sample(x as u32, y as u32);
            if c == 0 {
                continue;
            }
            let i = index(x, y);
            let before = pixels[i];
            let after = blend(before, color, c, erase, alpha_lock);
            changed |= before != after;
            pixels[i] = after;
        }
    }
    let mut state = doc.snapshot();
    if changed {
        state.layers[layer].pixels = pixels;
        state.layers[layer].placement = if extent == old {
            raster.placement
        } else {
            Some(extent)
        };
    }
    Ok(state)
}
fn blend(a: [u8; 4], b: [u8; 4], coverage: u8, erase: bool, alpha_lock: bool) -> [u8; 4] {
    if erase {
        return [
            a[0],
            a[1],
            a[2],
            ((a[3] as u32 * (255 - coverage as u32) + 127) / 255) as u8,
        ];
    }
    let sa = b[3] as f64 / 255.0 * coverage as f64 / 255.0;
    if sa == 0.0 || alpha_lock && a[3] == 0 {
        return a;
    }
    let da = a[3] as f64 / 255.0;
    let out = if alpha_lock { da } else { sa + da * (1.0 - sa) };
    let mut p = a;
    for n in 0..3 {
        p[n] = if alpha_lock {
            (a[n] as f64 * (1.0 - sa) + b[n] as f64 * sa).round() as u8
        } else {
            ((b[n] as f64 * sa + a[n] as f64 * da * (1.0 - sa)) / out).round() as u8
        };
    }
    p[3] = (out * 255.0).round() as u8;
    p
}

/// Generated fixtures also run from the packaged application's native smoke.
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    use crate::{
        document::{Background, CanvasSpec},
        scene::Rect,
        selection::{Mask, Run},
    };
    let make = |width, height| {
        Document::create(
            CanvasSpec {
                width,
                height,
                dpi: 300.0,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .map_err(|e| e.to_string())
    };
    let mut ring = make(7, 7)?;
    for y in 1..6 {
        for x in 1..6 {
            if x == 1 || x == 5 || y == 1 || y == 5 {
                ring.layers[0].pixels[y * 7 + x] = [0, 0, 0, 255];
            }
        }
    }
    let settings = Settings {
        antialias: false,
        ..Default::default()
    };
    let closed = Region::build(&ring, 0, [3.5, 3.5], settings)?;
    if closed.coverage(3, 3) != 255 || closed.coverage(0, 0) != 0 {
        return Err("Closed fill leaked".into());
    }
    ring.layers[0].pixels[7 + 3] = [0; 4];
    if Region::build(&ring, 0, [3.5, 3.5], settings)?.coverage(0, 0) != 255 {
        return Err("Strict fill unexpectedly closed gap".into());
    }
    let loose = Settings {
        mode: Mode::CloseGaps,
        gap: 1,
        ..settings
    };
    if Region::build(&ring, 0, [3.5, 3.5], loose)?.coverage(0, 0) != 0 {
        return Err("One-pixel gap was not closed".into());
    }
    let expanded = Region::build(&ring, 0, [3.5, 3.5], Settings { expand: 1, ..loose })?;
    if expanded.coverage(1, 3) != 255 || expanded.coverage(0, 0) != 0 {
        return Err("Fill expansion".into());
    }
    ring.layers[0].pixels[7 + 2] = [0; 4];
    ring.layers[0].pixels[7 + 4] = [0; 4];
    if Region::build(&ring, 0, [3.5, 3.5], loose)?.coverage(0, 0) != 255 {
        return Err("Large gap was silently closed".into());
    }
    let mut d = make(3, 1)?;
    d.layers[0].pixels = vec![[1, 2, 3, 0], [100, 200, 30, 127], [250, 50, 70, 128]];
    for mode in [Mode::Strict, Mode::CloseGaps] {
        for threshold in [
            127.0 / 255.0 - 1e-9,
            127.0 / 255.0,
            127.0 / 255.0 + 1e-9,
            128.0 / 255.0,
        ] {
            let s = Settings {
                mode,
                gap: 0,
                alpha_tolerance: threshold,
                antialias: false,
                ..settings
            };
            let r = Region::build(&d, 0, [0.5, 0.5], s)?;
            if r.coverage(1, 0) != if threshold >= 127.0 / 255.0 { 255 } else { 0 }
                || r.coverage(2, 0) != if threshold >= 128.0 / 255.0 { 255 } else { 0 }
            {
                return Err("Alpha threshold boundary".into());
            }
        }
    }
    let aa = Region::build(
        &d,
        0,
        [0.5, 0.5],
        Settings {
            antialias: true,
            ..settings
        },
    )?;
    if aa.coverage(1, 0) != 128 || aa.coverage(2, 0) != 0 {
        return Err("Antialias adjacency changed connectivity".into());
    }
    d.layers[0].pixels = vec![[100, 20, 30, 255], [101, 20, 30, 255], [130, 20, 30, 255]];
    if Region::build(&d, 0, [0.5, 0.5], settings)?.coverage(1, 0) != 0
        || Region::build(
            &d,
            0,
            [0.5, 0.5],
            Settings {
                color_tolerance: 1.0 / 255.0,
                ..settings
            },
        )?
        .coverage(1, 0)
            != 255
    {
        return Err("Independent color tolerance".into());
    }
    let mut d = make(3, 2)?;
    d.layers[0].pixels.fill([20, 30, 40, 128]);
    d.scene.selection_mask = Some(Mask {
        runs: vec![
            Run {
                x: 0,
                y: 0,
                length: 1,
                alpha: 128,
            },
            Run {
                x: 2,
                y: 1,
                length: 1,
                alpha: 255,
            },
        ],
    });
    let before = d.snapshot();
    let state = prepare(&d, 0, None, [200, 50, 80, 128], false, false, settings)?;
    if state.layers[0].pixels[0] != [92, 38, 56, 160]
        || state.layers[0].pixels[1] != [20, 30, 40, 128]
        || state.layers[0].pixels[5] != [140, 43, 67, 192]
    {
        return Err(format!(
            "Selection fill independent numeric result {:?} {:?}",
            state.layers[0].pixels[0], state.layers[0].pixels[5]
        ));
    }
    d.commit_state(state)?;
    if d.history_counts() != (1, 0) {
        return Err("Fill history".into());
    }
    let path = root.join("bucket-selected.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != d.snapshot()
    {
        return Err("Fill native reopen".into());
    }
    d.undo();
    if d.snapshot() != before {
        return Err("Fill Undo".into());
    }
    d.redo();
    d.undo();
    let erased = prepare(&d, 0, None, [0; 4], true, false, settings)?;
    if erased.layers[0].pixels[0] != [20, 30, 40, 64]
        || erased.layers[0].pixels[5] != [20, 30, 40, 0]
    {
        return Err("Transparent fill did not preserve RGB/selection".into());
    }
    let locked = prepare(&d, 0, None, [200, 0, 0, 255], false, true, settings)?;
    if locked.layers[0].pixels[0][3] != 128 {
        return Err("Alpha-lock fill changed alpha".into());
    }
    d.scene.selection_mask = Some(Mask::default());
    if prepare(&d, 0, None, [20; 4], false, false, settings)? != d.snapshot() {
        return Err("Empty selection fill".into());
    }
    d.scene.selection_mask = None;
    d.layers[0].placement = Some(PlacedRaster {
        width: 3,
        height: 2,
        offset: [-2, 0],
    });
    let outside = d.layers[0].pixel(3, 2, -2, 0);
    let state = prepare(&d, 0, None, [255, 0, 0, 255], false, false, settings)?;
    if state.layers[0].pixel(3, 2, -2, 0) != outside
        || state.layers[0].pixel(3, 2, 2, 1) != [255, 0, 0, 255]
    {
        return Err("Fill outside preservation/extension".into());
    }
    d.layers[0].locked = true;
    if prepare(&d, 0, None, [20; 4], false, false, settings).is_ok() {
        return Err("Locked fill accepted".into());
    }
    d.layers[0].locked = false;
    d.scene.selection = Some(Rect {
        min: [0.0; 2],
        max: [1.0; 2],
    });
    if prepare(&d, 99, None, [20; 4], false, false, settings).is_ok()
        || prepare(&d, 0, Some([-1.0, 0.0]), [20; 4], false, false, settings).is_ok()
        || prepare(
            &d,
            0,
            None,
            [20; 4],
            false,
            false,
            Settings {
                alpha_tolerance: f64::NAN,
                ..settings
            },
        )
        .is_ok()
    {
        return Err("Invalid fill accepted".into());
    }
    let mut references = make(7, 7)?;
    references.layers.push(ring.layers[0].clone());
    let active = Region::build(&references, 0, [3.5, 3.5], settings)?;
    // Reclose the generated large gap; no original application resources are used.
    for x in 2..5 {
        references.layers[1].pixels[7 + x] = [0, 0, 0, 255];
    }
    let other = Region::build(
        &references,
        0,
        [3.5, 3.5],
        Settings {
            reference: Reference::Layer(1),
            ..settings
        },
    )?;
    let visible = Region::build(
        &references,
        0,
        [3.5, 3.5],
        Settings {
            reference: Reference::Composite,
            ..settings
        },
    )?;
    if active.coverage(0, 0) != 255 || other.coverage(0, 0) != 0 || visible.coverage(0, 0) != 0 {
        return Err("Fill reference independence".into());
    }
    let mut snapshot = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    snapshot.input.fills[0] = loose;
    let p = crate::brush_presets::Preset::capture(
        &snapshot,
        crate::assets::BrushTip::Pen,
        "Fill preset",
    )?;
    let restored = crate::brush_presets::Preset::decode(&p.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        .apply(&crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        })?;
    if restored.input.fills[0] != loose {
        return Err("Fill preset persistence".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn threshold_gap_color_selection_alpha_undo_limits_and_preset() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
