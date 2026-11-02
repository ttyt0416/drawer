//! Local editing cameras over one shared document scene; guides never enter output.
use crate::{
    document::Document,
    scene::{Camera, Point},
    world_math::{V, add, dot, mul, sub},
    world3d::{self, Lens, Matrix, Projection},
};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub camera: Camera,
    pub lens: Lens,
}
impl View {
    pub fn valid(&self) -> bool {
        world3d::camera_valid(&self.camera) && self.lens.valid()
    }
    pub fn scene(d: &Document) -> Self {
        let (camera, lens) = world3d::default_camera(d);
        Self { camera, lens }
    }
    pub fn direction(&mut self, direction: Direction, target: V) {
        let delta = sub(self.camera.position, target);
        let distance = dot(delta, delta).sqrt().clamp(1., 1e7);
        self.lens.angles = match direction {
            Direction::Perspective => [-25., 35., 0.],
            Direction::Front => [0.; 3],
            Direction::Back => [0., 180., 0.],
            Direction::Left => [0., -90., 0.],
            Direction::Right => [0., 90., 0.],
            Direction::Top => [-90., 0., 0.],
            Direction::Bottom => [90., 0., 0.],
        };
        self.lens.projection = if direction == Direction::Perspective {
            Projection::Perspective
        } else {
            Projection::Orthographic
        };
        self.camera.position = add(
            target,
            Matrix::euler(self.lens.angles).vector([0., 0., distance]),
        );
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Perspective,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}
impl Direction {
    pub const ALL: [Self; 7] = [
        Self::Perspective,
        Self::Front,
        Self::Back,
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Perspective => "Perspective",
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub views: Vec<View>,
    pub selected: usize,
    pub reference: usize,
    pub linked: bool,
    pub count: usize,
    pub split: f32,
    pub camera_guides: bool,
    pub edges: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            views: vec![],
            selected: 0,
            reference: 0,
            linked: false,
            count: 2,
            split: 0.5,
            camera_guides: true,
            edges: false,
        }
    }
}
impl Preferences {
    pub fn valid(&self) -> bool {
        self.version == 1
            && matches!(self.count, 2 | 4)
            && self.selected < self.count
            && self.reference < self.count
            && self.split.is_finite()
            && (0.2..=0.8).contains(&self.split)
            && (self.views.is_empty()
                || self.views.len() == 4 && self.views.iter().all(View::valid))
            && (!self.linked
                || self.views.len() == 4
                    && self.views.iter().all(|v| *v == self.views[self.reference]))
    }
    pub fn initialize(&mut self, d: &Document) {
        if self.views.is_empty() {
            let view = View::scene(d);
            self.views = vec![view; 4];
            let target = target(d);
            for (i, dir) in [
                Direction::Perspective,
                Direction::Front,
                Direction::Right,
                Direction::Top,
            ]
            .into_iter()
            .enumerate()
            {
                self.views[i].direction(dir, target);
            }
        }
    }
    pub fn link(&mut self, reference: usize) -> Result<(), String> {
        if reference >= self.count || self.views.len() != 4 {
            return Err("Choose an existing reference camera before linking".into());
        }
        let view = self.views[reference].clone();
        self.views.fill(view);
        self.reference = reference;
        self.linked = true;
        Ok(())
    }
    pub fn replace(&mut self, index: usize, view: View) -> Result<(), String> {
        if index >= self.count || self.views.len() != 4 || !view.valid() {
            return Err("Invalid editing camera; previous view preserved".into());
        }
        if self.linked {
            self.views.fill(view);
            self.reference = index;
        } else {
            self.views[index] = view;
        }
        Ok(())
    }
    pub fn reset_split(&mut self) {
        self.count = 2;
        self.split = 0.5;
        self.selected = 0;
        self.reference = 0;
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local camera-view preferences".into());
        }
        let path = root.join("settings/world-views.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let p: Self = serde_json::from_slice(&crate::book_io::read_local(&path, 65536)?)
            .map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid camera-view preferences; file preserved".into());
        }
        Ok(p)
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        use std::io::Write;
        if !self.valid() || !crate::font_library::local(root) {
            return Err("Invalid local camera-view preferences".into());
        }
        let folder = root.join("settings");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let path = folder.join("world-views.json");
        let mut stage = crate::animation_export::Stage::new(&folder)?;
        if path.exists() {
            let old = crate::book_io::read_local(&path, 65536)?;
            let previous = serde_json::from_slice::<Self>(&old).is_ok_and(|p| p.valid());
            let name = if previous {
                "world-views.previous.json"
            } else {
                "world-views.damaged.json"
            };
            let mut f = stage.create(name)?;
            f.write_all(&old)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            drop(f);
            std::fs::rename(stage.directory.join(name), folder.join(name))
                .map_err(|e| e.to_string())?;
        }
        let mut f = stage.create("world-views.json")?;
        f.write_all(&serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(stage.directory.join("world-views.json"), path).map_err(|e| e.to_string())
    }
}
pub fn target(d: &Document) -> V {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for m in &d.scene.worlds {
        for p in &m.vertices {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    if lo[0].is_finite() {
        mul(add(lo, hi), 0.5)
    } else {
        [0.; 3]
    }
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub guides: Vec<(Point, Point, bool)>,
}
fn guide_lines(d: &Document, view: &View, cameras: bool, edges: bool) -> Vec<(Point, Point, bool)> {
    let mut lines = vec![];
    let normalized = |p: Point| {
        std::array::from_fn(|i| {
            (p[i] - view.camera.page_bounds.min[i])
                / (view.camera.page_bounds.max[i] - view.camera.page_bounds.min[i])
        })
    };
    if edges {
        for (i, _) in d.scene.worlds.iter().enumerate() {
            let key = crate::layer_search::Key::World(d.scene.catalog.worlds.id(i));
            if crate::layer_catalog::properties(d, key).hidden {
                continue;
            }
            let transform = d
                .scene
                .transform
                .then(crate::camera2d::display_matrix(d, key, false).0);
            for line in world3d::segments(&d.scene, i) {
                lines.push((
                    normalized(transform.map(line[0])),
                    normalized(transform.map(line[1])),
                    false,
                ));
            }
        }
    }
    if cameras {
        let source = View::scene(d);
        let rotation = Matrix::euler(source.lens.angles);
        let depth = 1.;
        let height = match source.lens.projection {
            Projection::Perspective => (source.lens.fov.to_radians() / 2.).tan() * depth,
            Projection::Orthographic => {
                (source.camera.page_bounds.max[1] - source.camera.page_bounds.min[1])
                    / source.camera.pixels_per_unit
                    / 2.
            }
        };
        let aspect = (source.camera.page_bounds.max[0] - source.camera.page_bounds.min[0])
            / (source.camera.page_bounds.max[1] - source.camera.page_bounds.min[1]);
        let points = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]].map(|p| {
            add(
                source.camera.position,
                rotation.vector([p[0] * height * aspect, p[1] * height, -depth]),
            )
        });
        for i in 0..4 {
            for (a, b) in [
                (points[i], points[(i + 1) % 4]),
                (source.camera.position, points[i]),
            ] {
                if let Some([a, b]) = view.lens.segment(&view.camera, a, b) {
                    lines.push((normalized(a), normalized(b), true));
                }
            }
        }
    }
    lines
}
pub fn render(
    d: &Document,
    p: &Preferences,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Frame>, String> {
    crate::book_io::check_cancel(cancel)?;
    if !p.valid() || p.views.len() != 4 {
        return Err("Initialize valid editing cameras first".into());
    }
    if p.edges && d.scene.worlds.iter().map(|m| m.edges.len()).sum::<usize>() > 65536 {
        return Err(
            "Face-edge guides exceed65536 segments; disable guides to render this scene".into(),
        );
    }
    let guide_memory = if p.edges {
        d.scene.worlds.iter().map(|m| m.edges.len()).sum::<usize>() * 48 * p.count
    } else {
        0
    };
    let triangles = d
        .scene
        .world3d
        .surfaces
        .values()
        .map(|s| s.triangles.len())
        .sum::<usize>();
    if d.memory_bytes()
        .saturating_mul(3)
        .saturating_add(triangles * 512)
        .saturating_add(2 * 1024 * 1024)
        .saturating_add(guide_memory)
        > budget
        || d.scene.worlds.len().saturating_mul(128 * 128 * p.count) > 32_000_000
    {
        return Err("Multiple-camera preview exceeds memory/ray-work budget".into());
    }
    let mut frames = vec![];
    for view in p.views.iter().take(p.count) {
        crate::book_io::check_cancel(cancel)?;
        let mut frozen = crate::layer_merge::preview(&d.snapshot(), d.current_frame)?;
        frozen.keyframe_content = d.keyframe_content;
        frozen.scene.camera = Some(view.camera.clone());
        frozen.scene.world3d.lens = Some(view.lens);
        let mut frame = Frame {
            width: 128,
            height: 128,
            pixels: vec![],
            guides: vec![],
        };
        let size = std::array::from_fn::<_, 2, _>(|i| {
            view.camera.page_bounds.max[i] - view.camera.page_bounds.min[i]
        });
        let maximum = size[0].max(size[1]);
        frame.width = (128. * size[0] / maximum).round().max(1.) as u32;
        frame.height = (128. * size[1] / maximum).round().max(1.) as u32;
        let renderer = crate::render::Renderer::new(&frozen);
        if let Some(e) = renderer.error() {
            return Err(e.into());
        }
        let keys = crate::render_order::normalized(&frozen.scene)
            .into_iter()
            .map(|e| e.key)
            .filter(|k| matches!(k, crate::layer_search::Key::World(_)))
            .collect::<Vec<_>>();
        for y in 0..frame.height {
            crate::book_io::check_cancel(cancel)?;
            for x in 0..frame.width {
                let point = [
                    view.camera.page_bounds.min[0]
                        + (x as f64 + 0.5) / frame.width as f64 * size[0],
                    view.camera.page_bounds.min[1]
                        + (y as f64 + 0.5) / frame.height as f64 * size[1],
                ];
                let mut color = crate::paper::Color::default();
                for key in &keys {
                    let mut source = crate::paper::Color::default();
                    source.add(renderer.source_pixel(*key, point));
                    crate::layer_catalog::properties(&frozen, *key)
                        .blend
                        .over(&mut color, source);
                }
                frame.pixels.extend(color.rgba());
            }
        }
        frame.guides = guide_lines(&frozen, view, false, p.edges);
        // The source camera is the real document camera, not the temporary preview camera.
        if p.camera_guides {
            frame.guides.extend(guide_lines(d, view, true, false));
        }
        frames.push(frame);
    }
    Ok(frames)
}
pub fn copy_to_scene(
    d: &Document,
    view: &View,
    budget: usize,
) -> Result<world3d::Prepared, String> {
    if !view.valid() {
        return Err("Invalid editing camera".into());
    }
    world3d::prepare(
        d,
        world3d::Edit::Camera {
            camera: view.camera.clone(),
            lens: view.lens,
        },
        budget,
        &AtomicBool::new(false),
    )
}
