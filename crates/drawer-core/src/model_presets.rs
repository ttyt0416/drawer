//! Local original-format pose, shape and camera assets, independent of document history.
use crate::{
    document::Document,
    human_rig::{self, Kind, Rig, Shape},
    scene::Camera,
    world3d::{self, Lens, V},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Pose,
    Shape,
    Camera,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skeleton {
    pub version: u32,
    pub kind: Kind,
    pub joints: Vec<(String, Option<usize>)>,
}
impl Skeleton {
    fn of(r: &Rig) -> Self {
        Self {
            version: r.version,
            kind: r.kind,
            joints: r.joints().into_iter().map(|j| (j.name, j.parent)).collect(),
        }
    }
    fn valid(&self) -> bool {
        *self == Self::of(&Rig::new(self.kind))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    pub skeleton: Skeleton,
    pub angles: Vec<V>,
    pub mirrored: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyShape {
    pub skeleton: Skeleton,
    pub shape: Shape,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub camera: Camera,
    pub lens: Lens,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Pose(Pose),
    Shape(BodyShape),
    Camera(View),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub version: u32,
    pub name: String,
    pub value: Value,
}
impl Preset {
    pub fn scope(&self) -> Scope {
        match self.value {
            Value::Pose(_) => Scope::Pose,
            Value::Shape(_) => Scope::Shape,
            Value::Camera(_) => Scope::Camera,
        }
    }
    pub fn valid(&self) -> bool {
        self.version == 1
            && !self.name.trim().is_empty()
            && self.name.chars().count() <= 80
            && self.name.len() <= 512
            && !self.name.chars().any(char::is_control)
            && match &self.value {
                Value::Pose(p) => {
                    let mut r = Rig::new(p.skeleton.kind);
                    r.angles = p.angles.clone();
                    r.mirrored = p.mirrored;
                    p.skeleton.valid() && r.valid()
                }
                Value::Shape(p) => p.skeleton.valid() && p.shape.valid(),
                Value::Camera(p) => world3d::camera_valid(&p.camera) && p.lens.valid(),
            }
    }
    pub fn capture(
        d: &Document,
        id: Option<u64>,
        scope: Scope,
        name: String,
    ) -> Result<Self, String> {
        let value = if scope == Scope::Camera {
            Value::Camera(View {
                camera: d
                    .scene
                    .camera
                    .clone()
                    .ok_or("Set a document3D camera first")?,
                lens: d
                    .scene
                    .world3d
                    .lens
                    .ok_or("Set a document3D camera first")?,
            })
        } else {
            let r = d
                .scene
                .world3d
                .rigs
                .get(&id.ok_or("Select a rigged mannequin")?)
                .ok_or("This model has no rig; pose / shape preset unavailable")?;
            if scope == Scope::Pose {
                Value::Pose(Pose {
                    skeleton: Skeleton::of(r),
                    angles: r.angles.clone(),
                    mirrored: r.mirrored,
                })
            } else {
                Value::Shape(BodyShape {
                    skeleton: Skeleton::of(r),
                    shape: r.shape,
                })
            }
        };
        let p = Self {
            version: 1,
            name,
            value,
        };
        if !p.valid() {
            return Err("Invalid preset name / values".into());
        }
        Ok(p)
    }
    pub fn prepare(
        &self,
        d: &Document,
        id: Option<u64>,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<world3d::Prepared, String> {
        if !self.valid() {
            return Err("Invalid preset version, skeleton or values".into());
        }
        let mut p = match &self.value {
            Value::Camera(v) => world3d::prepare(
                d,
                world3d::Edit::Camera {
                    camera: v.camera.clone(),
                    lens: v.lens,
                },
                budget,
                cancel,
            )?,
            Value::Pose(_) | Value::Shape(_) => {
                let id = id.ok_or("Select a destination mannequin")?;
                let mut r = d
                    .scene
                    .world3d
                    .rigs
                    .get(&id)
                    .cloned()
                    .ok_or("This model has no rig; pose / shape preset unavailable")?;
                let skeleton = match &self.value {
                    Value::Pose(v) => &v.skeleton,
                    Value::Shape(v) => &v.skeleton,
                    _ => unreachable!(),
                };
                if *skeleton != Skeleton::of(&r) {
                    return Err("Skeleton mismatch: automatic mapping is unavailable; original pose preserved".into());
                }
                match &self.value {
                    Value::Pose(v) => {
                        r.angles = v.angles.clone();
                        r.mirrored = v.mirrored;
                    }
                    Value::Shape(v) => r.shape = v.shape,
                    _ => unreachable!(),
                };
                human_rig::prepare(d, human_rig::Edit::Replace { id, rig: r }, budget, cancel)?
            }
        };
        p.summary = format!(
            "Preset {} / {:?}: preview before Apply",
            self.name,
            self.scope()
        );
        Ok(p)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u64,
    pub preset: Preset,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub version: u32,
    pub next: u64,
    pub entries: Vec<Entry>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            next: 1,
            entries: vec![],
        }
    }
}
impl Library {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.next > 0
            && self.entries.len() <= 128
            && self
                .entries
                .iter()
                .all(|e| e.id > 0 && e.id < self.next && e.preset.valid())
            && self
                .entries
                .iter()
                .map(|e| e.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.entries.len()
    }
    pub fn add(&mut self, preset: Preset) -> Result<u64, String> {
        if !self.valid() || !preset.valid() || self.entries.len() >= 128 {
            return Err("Invalid preset / library or limit128 reached".into());
        }
        let id = self.next;
        self.next = self.next.checked_add(1).ok_or("Preset ID limit")?;
        self.entries.push(Entry { id, preset });
        Ok(id)
    }
    pub fn remove(&mut self, id: u64) -> Result<(), String> {
        let i = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or("Preset removed")?;
        self.entries.remove(i);
        Ok(())
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        if !crate::font_library::local(root) {
            return Err("Choose local preset storage".into());
        }
        let p = root.join("settings/model-presets.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let p: Self = serde_json::from_slice(&crate::book_io::read_local(&p, 1024 * 1024)?)
            .map_err(|e| e.to_string())?;
        if !p.valid() {
            return Err("Invalid preset library; original file preserved".into());
        }
        Ok(p)
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        use std::io::Write;
        if !self.valid() || !crate::font_library::local(root) {
            return Err("Invalid local preset library".into());
        }
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if data.len() > 1024 * 1024 {
            return Err("Preset library exceeds1MiB".into());
        }
        let dir = root.join("settings");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut stage = crate::animation_export::Stage::new(&dir)?;
        let mut file = stage.create("model-presets.json")?;
        file.write_all(&data)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(
            stage.directory.join("model-presets.json"),
            dir.join("model-presets.json"),
        )
        .map_err(|e| e.to_string())
    }
}
