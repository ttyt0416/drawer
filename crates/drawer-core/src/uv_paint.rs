//! Local opaque RGB diffuse textures, projected painting and bounded UV diagnostics.
use crate::{document::Document, model_mesh::Surface, scene::Point, world3d};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::AtomicBool,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Texture {
    pub version: u32,
    pub size: u16,
    pub png: String,
}
impl Texture {
    pub fn encode(size: u16, pixels: &[u8]) -> Result<Self, String> {
        use image::ImageEncoder;
        if !valid_size(size)
            || pixels.len() != size as usize * size as usize * 4
            || pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 255)
        {
            return Err("Invalid opaque RGB texture".into());
        }
        let mut bytes = vec![];
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                pixels,
                size as u32,
                size as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            version: 1,
            size,
            png: STANDARD.encode(bytes),
        })
    }
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        if self.version != 1 || !valid_size(self.size) || self.png.len() > 6 * 1024 * 1024 {
            return Err("Invalid texture metadata / encoded size".into());
        }
        let bytes = self.png_bytes()?;
        if bytes.len() < 29 || bytes[24] != 8 || bytes[25] != 6 {
            return Err("Expected an RGBA8 PNG texture".into());
        }
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| e.to_string())?;
        if image.width() != self.size as u32 || image.height() != self.size as u32 {
            return Err("Texture dimensions mismatch".into());
        }
        let p = image.to_rgba8().into_raw();
        if p.as_chunks::<4>().0.iter().any(|p| p[3] != 255) {
            return Err("This diffuse paint texture requires opaque RGB pixels".into());
        }
        Ok(p)
    }
    pub fn png_bytes(&self) -> Result<Vec<u8>, String> {
        STANDARD.decode(&self.png).map_err(|e| e.to_string())
    }
    pub fn bytes(&self) -> usize {
        self.png.len() + self.size as usize * self.size as usize * 4
    }
}
fn valid_size(s: u16) -> bool {
    (64..=1024).contains(&s) && s.is_power_of_two()
}
#[derive(Clone, Copy)]
pub struct Brush {
    pub size: u16,
    pub radius: f64,
    pub opacity: f64,
    pub rgb: [u8; 3],
}
impl Default for Brush {
    fn default() -> Self {
        Self {
            size: 256,
            radius: 8.,
            opacity: 1.,
            rgb: [220, 45, 45],
        }
    }
}
impl Brush {
    fn valid(self) -> bool {
        valid_size(self.size)
            && self.radius.is_finite()
            && (0.5..=64.).contains(&self.radius)
            && self.opacity.is_finite()
            && (0. ..=1.).contains(&self.opacity)
    }
}
fn area(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn overlap(a: [Point; 3], b: [Point; 3]) -> bool {
    let sign = area(b[0], b[1], b[2]).signum();
    if sign == 0. {
        return false;
    }
    let mut p = a.to_vec();
    for i in 0..3 {
        let u = b[i];
        let v = b[(i + 1) % 3];
        let mut out = vec![];
        for j in 0..p.len() {
            let x = p[j];
            let y = p[(j + 1) % p.len()];
            let dx = area(u, v, x) * sign;
            let dy = area(u, v, y) * sign;
            if dx >= 0. {
                out.push(x);
            }
            if (dx >= 0.) != (dy >= 0.) {
                let t = dx / (dx - dy);
                out.push([x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t]);
            }
        }
        p = out;
        if p.len() < 3 {
            return false;
        }
    }
    let sum = (0..p.len())
        .map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[i][1] * p[(i + 1) % p.len()][0])
        .sum::<f64>();
    sum.abs() > 1e-9
}
pub fn inspect(s: &Surface) -> Result<Vec<String>, String> {
    let triangles = s
        .triangles
        .iter()
        .filter_map(|t| {
            t.t.map(|i| i.map(|i| [s.texcoords[i as usize][0], s.texcoords[i as usize][1]]))
        })
        .collect::<Vec<_>>();
    if triangles.is_empty() {
        return Err("This model has no UV coordinates; surface painting is unavailable".into());
    }
    let mut warnings = vec![];
    if s.triangles.iter().any(|t| t.t.is_none()) {
        warnings.push("Some faces have no UV; those faces cannot be painted".into());
    }
    let mut cells: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    let mut tested = BTreeSet::new();
    let mut work = 0;
    for (i, t) in triangles.iter().enumerate() {
        if t.iter().flatten().any(|v| !(0. ..=1.).contains(v)) {
            warnings.push(
                "UV outside the unit tile repeats; painting may affect shared / wrapped regions"
                    .into(),
            );
            return Ok(warnings);
        }
        let lo = std::array::from_fn::<_, 2, _>(|a| {
            (t.iter().map(|p| p[a]).fold(1., f64::min) * 31.).floor() as i32
        });
        let hi = std::array::from_fn::<_, 2, _>(|a| {
            (t.iter().map(|p| p[a]).fold(0., f64::max) * 31.).floor() as i32
        });
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let cell = cells.entry((x, y)).or_default();
                for &j in cell.iter() {
                    work += 1;
                    if work > 100_000 {
                        warnings.push("UV overlap analysis reached its work limit; review shared regions before applying".into());
                        return Ok(warnings);
                    }
                    if tested.insert((j, i)) && overlap(triangles[j], *t) {
                        warnings.push("Overlapping UV faces share texture pixels; painting one may recolor another".into());
                        return Ok(warnings);
                    }
                }
                cell.push(i);
            }
        }
    }
    Ok(warnings)
}
fn continuous(s: &Surface, a: usize, b: usize) -> bool {
    if a == b {
        return true;
    }
    let a = &s.triangles[a];
    let b = &s.triangles[b];
    let (Some(ta), Some(tb)) = (a.t, b.t) else {
        return false;
    };
    let mut shared = 0;
    for (i, &u) in ta.iter().enumerate() {
        for (j, &v) in tb.iter().enumerate() {
            if a.v[i] == b.v[j] && s.texcoords[u as usize] == s.texcoords[v as usize] {
                shared += 1;
            }
        }
    }
    shared >= 2
}
fn stamp(p: &mut [u8], s: usize, uv: Point, b: Brush) {
    let x = uv[0].rem_euclid(1.) * s as f64 - 0.5;
    let y = (1. - uv[1].rem_euclid(1.)) * s as f64 - 0.5;
    let lo = [(x - b.radius).floor() as i32, (y - b.radius).floor() as i32];
    let hi = [(x + b.radius).ceil() as i32, (y + b.radius).ceil() as i32];
    let delta = |v: f64| (v + s as f64 / 2.).rem_euclid(s as f64) - s as f64 / 2.;
    for iy in lo[1]..=hi[1].min(lo[1] + s as i32 - 1) {
        for ix in lo[0]..=hi[0].min(lo[0] + s as i32 - 1) {
            let coverage = (b.radius + 0.5 - delta(ix as f64 - x).hypot(delta(iy as f64 - y)))
                .clamp(0., 1.)
                * b.opacity;
            if coverage == 0. {
                continue;
            }
            let i = (iy.rem_euclid(s as i32) as usize * s + ix.rem_euclid(s as i32) as usize) * 4;
            for c in 0..3 {
                p[i + c] =
                    (p[i + c] as f64 * (1. - coverage) + b.rgb[c] as f64 * coverage).round() as u8;
            }
        }
    }
}
pub fn prepare(
    d: &Document,
    id: u64,
    points: &[Point],
    brush: Brush,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<world3d::Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if !brush.valid()
        || points.is_empty()
        || points.len() > 8192
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
    {
        return Err("Invalid projected brush / points".into());
    }
    crate::model_library::editable(d, &BTreeSet::from([id]))?;
    let surface = d
        .scene
        .world3d
        .surfaces
        .get(&id)
        .ok_or("This model has no UV surface")?;
    let size = surface.paint.as_ref().map_or(brush.size, |t| t.size);
    let memory = size as usize * size as usize * 4;
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    if triangles > 262144
        || d.memory_bytes()
            .saturating_mul(3)
            .saturating_add(memory * 6)
            .saturating_add(triangles * 512)
            .saturating_add(16 * 1024 * 1024)
            > budget
    {
        return Err("Texture / projected geometry memory budget exceeded".into());
    }
    let mut warnings = inspect(surface)?;
    if surface
        .imported
        .as_ref()
        .is_some_and(|a| a.materials.values().any(|m| m.image.is_some()))
    {
        warnings.push("RGB painting replaces imported diffuse RGB maps for this model; original embedded maps and imported alpha coverage are retained".into());
    }
    if brush.opacity == 0. {
        return world3d::prepared(
            d,
            d.snapshot(),
            "Zero-opacity texture stroke; unchanged".into(),
            vec![id],
            warnings,
        );
    }
    let mut pixels = if let Some(t) = &surface.paint {
        t.decode()?
    } else {
        vec![255; memory]
    };
    let object = d
        .scene
        .catalog
        .worlds
        .index(id, d.scene.worlds.len())
        .ok_or("Model removed")?;
    let trace = crate::world_render::Trace::paint_view(d);
    let transform = crate::camera2d::display_matrix(d, crate::layer_search::Key::World(id), false)
        .0
        .inverse()
        .then(d.scene.transform.inverse());
    let mut previous: Option<(Point, usize)> = None;
    let mut last_screen = None;
    let mut hits = 0;
    let mut work = 0usize;
    let mut paint_work = 0usize;
    for &point in points {
        let start = last_screen.unwrap_or(point);
        let distance = (point[0] - start[0]).hypot(point[1] - start[1]);
        let steps = distance.ceil().max(1.) as usize;
        work = work.saturating_add(steps);
        if work > 65536 {
            return Err("Projected stroke sampling limit exceeded".into());
        }
        for step in 1..=steps {
            crate::book_io::check_cancel(cancel)?;
            let f = step as f64 / steps as f64;
            let page: Point = std::array::from_fn(|i| start[i] + (point[i] - start[i]) * f);
            if page[0] < 0.
                || page[1] < 0.
                || page[0] >= d.width as f64
                || page[1] >= d.height as f64
            {
                previous = None;
                continue;
            }
            let p = transform.map(page);
            if let Some((uv, face)) = trace.uv(object, p) {
                if let Some((old, old_face)) = previous
                    && continuous(surface, old_face, face)
                {
                    let delta = std::array::from_fn::<_, 2, _>(|i| uv[i] - old[i]);
                    let length = delta[0].hypot(delta[1]) * size as f64;
                    let n = (length / (brush.radius / 2.).max(0.5)).ceil().max(1.) as usize;
                    if n > 4096 {
                        return Err("UV interpolation crosses an excessive span".into());
                    }
                    paint_work = paint_work.saturating_add(
                        n.saturating_mul(
                            ((brush.radius * 2. + 2.).ceil() as usize)
                                .min(size as usize)
                                .pow(2),
                        ),
                    );
                    if paint_work > 16_000_000 {
                        return Err("Texture stroke pixel-work budget exceeded".into());
                    }
                    for i in 1..=n {
                        stamp(
                            &mut pixels,
                            size as usize,
                            std::array::from_fn(|a| old[a] + delta[a] * i as f64 / n as f64),
                            brush,
                        );
                    }
                } else {
                    paint_work = paint_work.saturating_add(
                        ((brush.radius * 2. + 2.).ceil() as usize)
                            .min(size as usize)
                            .pow(2),
                    );
                    if paint_work > 16_000_000 {
                        return Err("Texture stroke pixel-work budget exceeded".into());
                    }
                    stamp(&mut pixels, size as usize, uv, brush);
                }
                previous = Some((uv, face));
                hits += 1;
            } else {
                previous = None;
            }
        }
        last_screen = Some(point);
    }
    if hits == 0 {
        return Err(
            "The projected stroke did not hit a visible UV face of the selected model".into(),
        );
    }
    let mut state = d.snapshot();
    state.scene.world3d.surfaces.get_mut(&id).unwrap().paint =
        Some(Texture::encode(size, &pixels)?);
    if state
        .scene
        .world3d
        .lighting
        .as_ref()
        .is_none_or(|s| s.style == crate::lighting::Style::Wire)
    {
        let mut settings = state.scene.world3d.lighting.clone().unwrap_or_default();
        settings.style = crate::lighting::Style::Unlit;
        state.scene.world3d.lighting = Some(settings);
        let (c, l) = world3d::default_camera(d);
        state.scene.camera = Some(c);
        state.scene.world3d.lens = Some(l);
    }
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Paint result exceeds memory budget".into());
    }
    let frozen = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::files::native_scene(&frozen)
        .map_err(|e| format!("Texture exceeds native document capacity: {e}"))?;
    crate::book_io::check_cancel(cancel)?;
    world3d::prepared(
        d,
        state,
        format!(
            "Projected RGB texture stroke: {hits} UV hits; {size}x{size}; wire views adopt Unlit"
        ),
        vec![id],
        warnings,
    )
}
pub fn export(d: &Document, id: u64, path: &Path) -> Result<(), String> {
    use std::io::Write;
    if !crate::font_library::local(path)
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err("Choose a local PNG output".into());
    }
    let t = d
        .scene
        .world3d
        .surfaces
        .get(&id)
        .and_then(|s| s.paint.as_ref())
        .ok_or("Paint a local texture first")?;
    t.decode()?;
    let directory = path.parent().ok_or("Choose an output directory")?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid output name")?;
    let mut stage = crate::animation_export::Stage::new(directory)?;
    let mut file = stage.create(name)?;
    file.write_all(&t.png_bytes()?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(stage.directory.join(name), path).map_err(|e| e.to_string())
}
