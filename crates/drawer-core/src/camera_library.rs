//! Named scene cameras, explicit lens framing and bounds fitting. Editing views stay local.
use crate::{
    document::Document,
    scene::Scene,
    world_math::{add, sub},
    world_views::View,
    world3d::{self, Matrix, Projection, V},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub saved: Option<View>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub next: u64,
    pub active: u64,
    pub entries: Vec<Entry>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            next: 1,
            active: 0,
            entries: vec![],
        }
    }
}
fn name(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self, s: &Scene) -> bool {
        self.is_default()
            || self.entries.len() <= 32
                && !self.entries.is_empty()
                && self.next > 0
                && s.camera.is_some()
                && s.world3d.lens.is_some()
                && self
                    .entries
                    .iter()
                    .map(|e| e.id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.entries.len()
                && self.entries.iter().all(|e| {
                    e.id > 0
                        && e.id < self.next
                        && name(&e.name)
                        && if e.id == self.active {
                            e.saved.is_none()
                        } else {
                            e.saved.as_ref().is_some_and(View::valid)
                        }
                })
                && self.entries.iter().any(|e| e.id == self.active)
    }
    pub fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.name.len() + 512).sum()
    }
}
fn initialize(d: &Document, s: &mut Scene) {
    if s.world3d.cameras.is_default() {
        let v = View::scene(d);
        s.camera = Some(v.camera);
        s.world3d.lens = Some(v.lens);
        s.world3d.cameras = Data {
            next: 2,
            active: 1,
            entries: vec![Entry {
                id: 1,
                name: "Scene camera".into(),
                saved: None,
            }],
        };
    }
}
fn current(s: &Scene) -> View {
    View {
        camera: s.camera.clone().unwrap(),
        lens: s.world3d.lens.unwrap(),
    }
}
fn switch(s: &mut Scene, id: u64) -> Result<(), String> {
    if id == s.world3d.cameras.active {
        return Ok(());
    }
    let next = s
        .world3d
        .cameras
        .entries
        .iter()
        .position(|e| e.id == id)
        .ok_or("Camera removed")?;
    let view = s.world3d.cameras.entries[next]
        .saved
        .take()
        .ok_or("Saved camera missing")?;
    let old = current(s);
    let active = s.world3d.cameras.active;
    s.world3d
        .cameras
        .entries
        .iter_mut()
        .find(|e| e.id == active)
        .ok_or("Active camera missing")?
        .saved = Some(old);
    s.camera = Some(view.camera);
    s.world3d.lens = Some(view.lens);
    s.world3d.cameras.active = id;
    Ok(())
}
pub fn straighten(v: &View, target: V) -> Result<View, String> {
    if !v.valid() || target.into_iter().any(|x| !x.is_finite() || x.abs() > 1e9) {
        return Err("Choose a finite camera target".into());
    }
    let before = v
        .lens
        .project(&v.camera, target)
        .ok_or("Target is outside camera depth range")?;
    let mut out = v.clone();
    out.lens.angles[0] = 0.;
    out.lens.angles[2] = 0.;
    out.lens.vertical = true;
    out.lens.level = Some(world3d::LevelTarget {
        world: target,
        anchor: std::array::from_fn(|i| {
            (before[i] - v.camera.page_bounds.min[i])
                / (v.camera.page_bounds.max[i] - v.camera.page_bounds.min[i])
        }),
        shift_base: v.lens.shift,
    });
    if !out.valid() {
        return Err(
            "Cannot retain the target in front of this level camera within the lens shift range"
                .into(),
        );
    }
    Ok(out)
}
pub fn fit(
    d: &Document,
    v: &View,
    ids: &[u64],
    margin: f64,
    cancel: &AtomicBool,
) -> Result<View, String> {
    crate::book_io::check_cancel(cancel)?;
    if !v.valid()
        || !margin.is_finite()
        || !(0.02..=0.4).contains(&margin)
        || ids.is_empty()
        || ids.len() > 256
        || ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len()
    {
        return Err("Choose models, a valid camera and2..40% margin".into());
    }
    let rotation = Matrix::euler(v.lens.angles);
    let inverse = rotation.inverse();
    let posed = crate::pose_sequence::meshes(d)?;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for id in ids {
        crate::book_io::check_cancel(cancel)?;
        let i = d
            .scene
            .catalog
            .worlds
            .index(*id, d.scene.worlds.len())
            .ok_or("Fit target removed")?;
        let mesh = posed.get(id).unwrap_or(&d.scene.worlds[i]);
        for p in &mesh.vertices {
            let p = inverse.vector(*p);
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    if !lo[0].is_finite() {
        return Err("Fit selection has no vertices".into());
    }
    let middle = std::array::from_fn(|i| (lo[i] + hi[i]) / 2.);
    let target = rotation.vector(middle);
    let span = std::array::from_fn::<_, 3, _>(|i| hi[i] - lo[i]);
    let radius = span.into_iter().fold(0f64, f64::max).max(0.02);
    let corners = (0..8)
        .map(|n| {
            rotation.vector(std::array::from_fn(|i| {
                if n & (1 << i) == 0 { lo[i] } else { hi[i] }
            }))
        })
        .collect::<Vec<_>>();
    let size = std::array::from_fn::<_, 2, _>(|i| {
        v.camera.page_bounds.max[i] - v.camera.page_bounds.min[i]
    });
    let mut out = v.clone();
    if out.lens.vertical {
        out.lens.level = Some(world3d::LevelTarget {
            world: target,
            anchor: [0.5; 2],
            shift_base: out.lens.shift,
        });
    }
    let focal = size[1] / 2. / (v.lens.fov.to_radians() / 2.).tan();
    if out.lens.projection == Projection::Orthographic {
        out.camera.pixels_per_unit = ((size[0] * (1. - 2. * margin) / span[0].max(0.02))
            .min(size[1] * (1. - 2. * margin) / span[1].max(0.02)))
        .clamp(1e-6, 1e6);
    }
    let position = |distance: f64, ppu: f64| {
        let scale = if v.lens.projection == Projection::Perspective {
            distance / focal
        } else {
            1. / ppu
        };
        add(
            target,
            rotation.vector([
                v.lens.shift[0] * size[0] * scale,
                -v.lens.shift[1] * size[1] * scale,
                distance,
            ]),
        )
    };
    let fits = |distance: f64| {
        let mut c = out.camera.clone();
        c.position = position(distance, c.pixels_per_unit);
        let mut l = out.lens;
        l.near = 1e-9;
        l.far = 1e9;
        corners.iter().all(|p| {
            l.project(&c, *p).is_some_and(|p| {
                (0..2).all(|i| {
                    p[i] >= c.page_bounds.min[i] + size[i] * margin - 1e-8
                        && p[i] <= c.page_bounds.max[i] - size[i] * margin + 1e-8
                })
            })
        })
    };
    let mut distance = radius.max(1.);
    for _ in 0..64 {
        if fits(distance) {
            break;
        }
        distance *= 2.;
        if distance > 5e8 {
            return Err("Fit cannot stay within camera/clip limits".into());
        }
    }
    if !fits(distance) {
        return Err("Unable to frame all selected bounds".into());
    }
    if out.lens.projection == Projection::Perspective {
        let (mut low, mut high) = (0., distance);
        for _ in 0..48 {
            let mid = (low + high) / 2.;
            if fits(mid) {
                high = mid;
            } else {
                low = mid;
            }
        }
        distance = high * 1.000001;
    }
    out.camera.position = position(distance, out.camera.pixels_per_unit);
    let depths = corners
        .iter()
        .map(|p| -inverse.vector(sub(*p, out.camera.position))[2])
        .collect::<Vec<_>>();
    let near = depths.iter().copied().fold(f64::INFINITY, f64::min);
    let far = depths.iter().copied().fold(0f64, f64::max);
    out.lens.near = (near * 0.5).max(1e-9);
    out.lens.far = (far * 1.5).max(out.lens.near + 0.01);
    if !out.valid() {
        return Err("Fit exceeds camera position, scale or clipping limits".into());
    }
    Ok(out)
}
pub enum Edit {
    Enable,
    New {
        name: String,
        view: View,
    },
    Duplicate(String),
    Switch(u64),
    Delete(u64),
    Rename(String),
    View(View),
    Fit {
        view: View,
        ids: Vec<u64>,
        margin: f64,
    },
}
pub struct Prepared {
    inner: world3d::Prepared,
    frame: u32,
    pub summary: String,
}
impl Prepared {
    pub fn commit(self, d: &mut Document) -> Result<bool, String> {
        if self.frame != d.current_frame {
            return Err("Camera preview frame changed".into());
        }
        self.inner.commit(d)
    }
    pub fn thumbnail(
        &self,
        d: &Document,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, String> {
        let mut state = d.snapshot();
        state.scene = self.inner.scene().clone();
        {
            let mut frozen = crate::layer_merge::preview(&state, d.current_frame)?;
            frozen.keyframe_content = d.keyframe_content;
            render(&frozen, budget, cancel)
        }
    }
}
fn render(d: &Document, budget: usize, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(triangles.saturating_mul(512))
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Camera preview exceeds memory budget".into());
    }
    let renderer = crate::render::Renderer::new(d);
    if let Some(e) = renderer.error() {
        return Err(e.into());
    }
    let mut image = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128 {
        crate::book_io::check_cancel(cancel)?;
        for x in 0..128 {
            image.extend(
                renderer
                    .display_sample([
                        (x as f64 + 0.5) * d.width as f64 / 128.,
                        (y as f64 + 0.5) * d.height as f64 / 128.,
                    ])?
                    .rgba8(),
            );
        }
    }
    Ok(image)
}
pub fn prepare(
    d: &Document,
    e: Edit,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    crate::book_io::check_cancel(cancel)?;
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(4 * 1024 * 1024)
        > budget
    {
        return Err("Camera edit exceeds memory budget".into());
    }
    let mut state = d.snapshot();
    initialize(d, &mut state.scene);
    let s = &mut state.scene;
    let summary = match e {
        Edit::Enable => "Create scene camera list",
        Edit::New { name: n, view } => {
            if !name(&n) || !view.valid() || s.world3d.cameras.entries.len() >= 32 {
                return Err("Choose a valid camera/name and at most32 cameras".into());
            }
            let id = s.world3d.cameras.next;
            s.world3d.cameras.next = id.checked_add(1).ok_or("Camera ID exhausted")?;
            s.world3d.cameras.entries.push(Entry {
                id,
                name: n,
                saved: Some(view),
            });
            "Create independent named camera"
        }
        Edit::Duplicate(n) => {
            if !name(&n) || s.world3d.cameras.entries.len() >= 32 {
                return Err("Choose a name and at most32 cameras".into());
            }
            let view = current(s);
            let id = s.world3d.cameras.next;
            s.world3d.cameras.next = id.checked_add(1).ok_or("Camera ID exhausted")?;
            s.world3d.cameras.entries.push(Entry {
                id,
                name: n,
                saved: Some(view),
            });
            "Duplicate active camera independently"
        }
        Edit::Switch(id) => {
            switch(s, id)?;
            "Switch active scene camera"
        }
        Edit::Delete(id) => {
            if s.world3d.cameras.entries.len() == 1 {
                return Err("Keep at least one scene camera".into());
            }
            if !s.world3d.cameras.entries.iter().any(|e| e.id == id) {
                return Err("Camera removed".into());
            }
            if id == s.world3d.cameras.active {
                let next = s
                    .world3d
                    .cameras
                    .entries
                    .iter()
                    .find(|e| e.id != id)
                    .unwrap()
                    .id;
                switch(s, next)?;
            }
            s.world3d.cameras.entries.retain(|e| e.id != id);
            "Delete named camera; retain a valid active camera"
        }
        Edit::Rename(n) => {
            if !name(&n) {
                return Err("Choose a bounded camera name".into());
            }
            let id = s.world3d.cameras.active;
            s.world3d
                .cameras
                .entries
                .iter_mut()
                .find(|e| e.id == id)
                .unwrap()
                .name = n;
            "Rename active scene camera"
        }
        Edit::View(v) => {
            if !v.valid() {
                return Err(
                    "Invalid camera/lens shift; vertical mode requires zero pitch/roll".into(),
                );
            }
            s.camera = Some(v.camera);
            s.world3d.lens = Some(v.lens);
            "Edit active camera projection / lens framing"
        }
        Edit::Fit { view, ids, margin } => {
            let v = fit(d, &view, &ids, margin, cancel)?;
            s.camera = Some(v.camera);
            s.world3d.lens = Some(v.lens);
            "Fit active camera to selected posed model bounds with margin"
        }
    };
    if state.memory_bytes().saturating_mul(3) > budget {
        return Err("Camera list/Undo exceeds memory budget".into());
    }
    let frozen = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::files::native_scene(&frozen).map_err(|e| e.to_string())?;
    let inner = world3d::prepared(d, state, summary.into(), vec![], vec![])?.named(summary);
    Ok(Prepared {
        inner,
        frame: d.current_frame,
        summary: summary.into(),
    })
}
