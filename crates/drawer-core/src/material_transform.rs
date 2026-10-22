//! Single-source transform drafts. Original bitmaps stay immutable.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as c,
    layer_properties::{self as p, Target},
    layer_search::Key,
    material_layer::Layer,
    material_mapping::{Tiling, UNIT, Warp},
    scene::{Affine, PageObject, Point, Rect},
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    ScaleRotate,
    Scale,
    Rotate,
    Free,
    Skew,
    Distort,
    Perspective,
    Mesh,
}
impl Mode {
    pub const ALL: [Self; 8] = [
        Self::ScaleRotate,
        Self::Scale,
        Self::Rotate,
        Self::Free,
        Self::Skew,
        Self::Distort,
        Self::Perspective,
        Self::Mesh,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ScaleRotate => "Scale/Rotate",
            Self::Scale => "Scale",
            Self::Rotate => "Rotate",
            Self::Free => "Free Transform",
            Self::Skew => "Skew",
            Self::Distort => "Distort",
            Self::Perspective => "Perspective",
            Self::Mesh => "Mesh Transformation",
        }
    }
    pub fn nonlinear(self) -> bool {
        matches!(self, Self::Distort | Self::Perspective | Self::Mesh)
    }
}
#[derive(Clone, Debug)]
pub struct Settings {
    pub mode: Mode,
    pub pivot: Point,
    pub movement: Point,
    pub scale: Point,
    pub angle: f64,
    pub skew: Point,
    pub free: Affine,
    pub quad: [Point; 4],
    pub columns: u8,
    pub rows: u8,
    pub mesh: Vec<Point>,
    pub preserve_width: bool,
    pub repeat: [bool; 2],
    pub tiling: Tiling,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::ScaleRotate,
            pivot: [0.; 2],
            movement: [0.; 2],
            scale: [1.; 2],
            angle: 0.,
            skew: [0.; 2],
            free: Default::default(),
            quad: UNIT,
            columns: 3,
            rows: 3,
            mesh: (0..3)
                .flat_map(|y| (0..3).map(move |x| [x as f64 / 2., y as f64 / 2.]))
                .collect(),
            preserve_width: true,
            repeat: [false; 2],
            tiling: Tiling::Repeat,
        }
    }
}
fn material(d: &Document, key: Key) -> Option<&Layer> {
    d.scene.file_objects.get(key).map(|l| &l.cache).or_else(|| {
        matches!(key, Key::Object(_))
            .then(|| c::source_index(d, key))
            .flatten()
            .and_then(|n| d.scene.objects.get(n))
            .and_then(|o| {
                if let PageObject::Material(m) = o {
                    Some(m.as_ref())
                } else {
                    None
                }
            })
    })
}
pub fn targets(d: &Document) -> Result<Vec<(Key, String)>, String> {
    Ok(c::rows(d)?
        .into_iter()
        .filter(|r| material(d, r.key).is_some() || matches!(r.key, Key::Layer(Target::Vector(_))))
        .map(|r| (r.key, r.name))
        .collect())
}
pub fn settings(d: &Document, key: Key) -> Result<Settings, String> {
    let bounds =
        c::bounds(d, key).ok_or("Select an image material, linked source or vector layer")?;
    let mut s = Settings {
        pivot: [0, 1].map(|n| (bounds.min[n] + bounds.max[n]) / 2.),
        ..Default::default()
    };
    if let Some(m) = material(d, key) {
        s.repeat = m.repeat;
        s.tiling = m.mapping.tiling;
        match &m.mapping.warp {
            Warp::Distort(q) => s.quad = *q,
            Warp::Perspective(q) => s.quad = *q,
            Warp::Mesh {
                columns,
                rows,
                points,
            } => {
                s.columns = *columns;
                s.rows = *rows;
                s.mesh = points.clone();
            }
            _ => {}
        }
    }
    Ok(s)
}
impl Settings {
    pub fn matrix(&self) -> Result<Affine, String> {
        if self
            .pivot
            .iter()
            .chain(&self.movement)
            .any(|v| !v.is_finite() || v.abs() > 1e9)
            || self
                .scale
                .iter()
                .any(|v| !v.is_finite() || !(0.01..=100.).contains(v))
            || !self.angle.is_finite()
            || self.angle.abs() > 36000.
            || self.skew.iter().any(|v| !v.is_finite() || v.abs() > 85.)
        {
            return Err("Transform numeric range or pivot limit".into());
        }
        let (s, c) = self.angle.to_radians().sin_cos();
        let mut m = match self.mode {
            Mode::ScaleRotate => Affine([
                c * self.scale[0],
                s * self.scale[0],
                -s * self.scale[1],
                c * self.scale[1],
                0.,
                0.,
            ]),
            Mode::Scale => Affine([self.scale[0], 0., 0., self.scale[1], 0., 0.]),
            Mode::Rotate => Affine([c, s, -s, c, 0., 0.]),
            Mode::Free => self.free,
            Mode::Skew => Affine([
                1.,
                self.skew[1].to_radians().tan(),
                self.skew[0].to_radians().tan(),
                1.,
                0.,
                0.,
            ]),
            _ => Affine::default(),
        };
        if !m.valid() {
            return Err("Transform matrix is singular".into());
        }
        let mapped = m.vector(self.pivot);
        m.0[4] += self.pivot[0] - mapped[0] + self.movement[0];
        m.0[5] += self.pivot[1] - mapped[1] + self.movement[1];
        if !m.valid() {
            return Err("Transform coordinate limit".into());
        }
        Ok(m)
    }
    fn warp(&self, old: &Warp) -> Result<Warp, String> {
        let w = match self.mode {
            Mode::Distort => Warp::Distort(self.quad),
            Mode::Perspective => Warp::Perspective(self.quad),
            Mode::Mesh => Warp::Mesh {
                columns: self.columns,
                rows: self.rows,
                points: self.mesh.clone(),
            },
            _ => old.clone(),
        };
        if !w.valid() {
            return Err(
                "Transform folds, crosses its horizon, or exceeds the 5×5 mesh limit".into(),
            );
        }
        Ok(w)
    }
}
#[derive(Clone)]
struct Frame {
    area: Rect,
    transform: Affine,
    warp: Warp,
}
impl Frame {
    fn material(m: &Layer) -> Self {
        Self {
            area: m.area,
            transform: m.transform,
            warp: m.mapping.warp.clone(),
        }
    }
    fn forward(&self, p: Point) -> Option<Point> {
        let uv = self.warp.forward(p)?;
        Some(
            self.transform.map(
                [0, 1].map(|n| self.area.min[n] + uv[n] * (self.area.max[n] - self.area.min[n])),
            ),
        )
    }
    fn inverse(&self, p: Point) -> Option<Point> {
        let p = self.transform.inverse().map(p);
        self.warp.inverse(
            [0, 1].map(|n| (p[n] - self.area.min[n]) / (self.area.max[n] - self.area.min[n])),
        )
    }
    fn bounds(&self, scene: Affine) -> Result<Rect, String> {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in self.warp.points() {
            let p = scene.map(self.transform.map(
                [0, 1].map(|n| self.area.min[n] + point[n] * (self.area.max[n] - self.area.min[n])),
            ));
            for n in 0..2 {
                min[n] = min[n].min(p[n]);
                max[n] = max[n].max(p[n]);
            }
        }
        let r = Rect { min, max };
        if !r.valid() {
            return Err("Transformed source bounds are invalid".into());
        }
        Ok(r)
    }
}
pub struct Prepared {
    pub state: DocumentState,
    pub notes: Vec<String>,
    pub selected: Key,
}
pub fn prepare(d: &Document, key: Key, s: &Settings) -> Result<Prepared, String> {
    let target = c::target(d, key)
        .filter(|t| p::targets(d).contains(t))
        .ok_or("Missing transform target")?;
    let flags = crate::file_objects::stored_flags(d, target);
    if flags.hidden
        || flags.locked
        || flags.position_locked
        || d.scene.clipping.solo.is_some()
        || d.scene.layer_groups.solo.is_some()
    {
        return Err("Show and unlock the source position before transforming it".into());
    }
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Transform snapshot limit".into());
    }
    let delta = s.matrix()?;
    let original = d
        .scene
        .transform
        .then(delta)
        .then(d.scene.transform.inverse());
    let mut state = d.snapshot();
    let mut notes = vec![format!(
        "{} uses the document pivot ({:.2}, {:.2}). Source bitmap RGBA is preserved; one Undo restores every property and mask.",
        s.mode.label(),
        s.pivot[0],
        s.pivot[1]
    )];
    let (old_frame, new_frame) = if let Some(m) = material(d, key) {
        let old = Frame::material(m);
        let mut new = m.clone();
        new.transform = new.transform.then(original);
        new.repeat = s.repeat;
        new.mapping.tiling = s.tiling;
        new.mapping.warp = s.warp(&m.mapping.warp)?;
        if !new.valid(false) {
            return Err("Invalid transformed material or extent".into());
        }
        let frame = Frame::material(&new);
        if let Some(link) = state
            .scene
            .file_objects
            .entries
            .iter_mut()
            .find(|l| l.target == key)
        {
            link.cache = new;
        } else {
            let n = c::source_index(d, key).ok_or("Missing material source")?;
            state.scene.objects[n] = PageObject::Material(Box::new(new));
        }
        notes.push("Repeat translates tiles. Reverse mirrors alternate tiles independently along each repeated axis. Flip rotates checkerboard-alternate tiles by 180°. Axis switches remain independent.".into());
        (old, frame)
    } else if let Target::Vector(id) = target {
        let area = c::bounds(d, key).ok_or("Vector layer has no bounded paths")?;
        if (0..2).any(|n| area.max[n] - area.min[n] <= 1e-9) {
            return Err("Vector transform needs nonempty bounds".into());
        }
        let old = Frame {
            area,
            transform: d.scene.transform.inverse(),
            warp: Warp::None,
        };
        let new = Frame {
            transform: old.transform.then(original),
            warp: s.warp(&Warp::None)?,
            ..old.clone()
        };
        let determinant = delta.0[0] * delta.0[3] - delta.0[1] * delta.0[2];
        let width_scale = if s.preserve_width {
            1.
        } else {
            determinant.abs().sqrt()
        };
        let layer = state
            .scene
            .lines
            .layers
            .iter_mut()
            .find(|l| l.id == id)
            .ok_or("Missing vector layer")?;
        let mut work = 0usize;
        for path in &mut layer.paths {
            if s.mode.nonlinear() && path.svg.is_some() {
                return Err(
                    "Rasterize SVG gradient/clipping outlines before nonlinear deformation".into(),
                );
            }
            if let Some(svg) = &mut path.svg {
                svg.transform(original);
            }
            work += path.points.len();
            if work > 32768 {
                return Err("Vector transform exceeds 32768 points".into());
            }
            for point in &mut path.points {
                if s.mode.nonlinear() {
                    let uv = old
                        .inverse(*point)
                        .ok_or("Vector point is outside the transform domain")?;
                    *point = new.forward(uv).ok_or("Vector deformation failed")?;
                } else {
                    *point = original.map(*point);
                }
            }
            path.width *= width_scale;
            if s.mode.nonlinear() {
                path.definition = None;
            } else if let Some(def) = &mut path.definition {
                def.creation = original.inverse().then(def.creation);
            }
        }
        state.scene.lines.prune_selection();
        notes.push(if s.preserve_width{"Vector stroke widths remain unchanged. Nonlinear changes preserve editable path points and convert parameterized shapes to editable polylines."}else{"Affine vector widths scale by the square root of the matrix area factor. Nonlinear edits change path geometry while retaining its width."}.into());
        (old, new)
    } else {
        return Err("Choose an image material, linked source or vector layer".into());
    };
    if let Some(mask) = crate::layer_masks::get(d, target).filter(|m| m.linked) {
        let mut mask = mask.clone();
        if old_frame.warp == new_frame.warp {
            mask.space = mask.space.then(original);
        } else if !mask.coverage.runs.is_empty() {
            // The nonlinear image domain is finite. Retain the original mask
            // as an Undo snapshot, and resample its visible support alongside
            // that domain. The inverted outside strength remains unchanged.
            let bounds = new_frame.bounds(d.scene.transform)?;
            let mut sampling = mask.clone();
            sampling.enabled = true;
            let coverage = crate::layer_masks::raster(bounds.min, bounds.max, |point| {
                let p = d.scene.transform.inverse().map(point);
                let value = new_frame
                    .inverse(p)
                    .and_then(|uv| old_frame.forward(uv))
                    .map_or(if sampling.inverted { 1. } else { 0. }, |p| {
                        sampling.factor(p)
                    });
                let alpha = (value * 255.).round() as u8;
                if sampling.inverted {
                    255 - alpha
                } else {
                    alpha
                }
            })?;
            mask.coverage = coverage;
            mask.space = d.scene.transform.inverse();
            mask.offset = [0.; 2];
            notes.push("The linked mask is resampled on the deformed source bounds; mask detail outside the image domain is discarded. Review the preview; Undo restores the original mask. Unlink the mask to keep it fixed in page space.".into());
        }
        let effects = &mut state.scene.layer_effects;
        let effect = match target {
            Target::Raster(id) => effects.rasters.entry(id).or_default(),
            Target::Vector(id) => effects.vectors.entry(id).or_default(),
            Target::Folder(id) => effects.folders.entry(id).or_default(),
            _ => return Err("Unsupported mask owner".into()),
        };
        effect.mask = Some(mask);
    }
    if !s.mode.nonlinear() {
        let paper = match target {
            Target::Raster(id) => {
                crate::layers::index(d, id).and_then(|n| state.scene.paper.layers.get_mut(&n))
            }
            Target::Folder(n) => Some(&mut state.scene.paper.folders[n as usize].paper),
            _ => None,
        };
        if let Some(paper) = paper {
            paper.space = paper.space.then(original);
        }
        let style = match target {
            Target::Raster(id) => state.scene.appearance.rasters.get_mut(&id),
            Target::Vector(id) => state.scene.appearance.vectors.get_mut(&id),
            _ => None,
        };
        if let Some(style) = style {
            style.scope_space = style.scope_space.then(original);
        }
    }
    crate::file_objects::validate(d, &state)?;
    if !state.scene.valid()
        || !state.color.valid_layers(&state.layers)
        || state.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
    {
        return Err("Transform scene or Undo limit".into());
    }
    Ok(Prepared {
        state,
        notes,
        selected: key,
    })
}
