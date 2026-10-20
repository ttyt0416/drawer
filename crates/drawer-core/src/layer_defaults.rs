//! Local creation defaults contain selected property values, never document identities/content.
use crate::{
    appearance::Style,
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as catalog,
    layer_properties::{self as p, Properties, Target},
    layer_search::{Expression, Key, Kind},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Storage {
    Raster,
    Vector,
    Folder,
    Track,
    Cell,
    Object,
    Gradient,
    World,
    Selection,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Class {
    pub kind: Kind,
    pub storage: Storage,
}
impl Class {
    pub fn label(self) -> &'static str {
        match self.kind {
            Kind::RasterColor => "Raster (all expression colors)",
            Kind::VectorColor => "Vector (all expression colors)",
            kind => kind.label(),
        }
    }
    pub fn appearance(self) -> bool {
        matches!(self.storage, Storage::Raster | Storage::Vector) && self.kind != Kind::Correction
    }
    pub fn opacity(self) -> bool {
        self.storage != Storage::Selection
    }
    pub fn blend(self) -> bool {
        !matches!(self.storage, Storage::Cell | Storage::Selection)
    }
    fn valid(self) -> bool {
        !matches!(
            self.kind,
            Kind::RasterGray
                | Kind::RasterMono
                | Kind::VectorGray
                | Kind::VectorMono
                | Kind::Mask
                | Kind::Ruler
                | Kind::Draft
                | Kind::Broader
        )
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fields {
    pub appearance: bool,
    pub expression: bool,
    pub opacity: bool,
    pub blend: bool,
    pub color: bool,
}
impl Fields {
    pub fn supported(c: Class) -> Self {
        Self {
            appearance: c.appearance(),
            expression: c.appearance(),
            opacity: c.opacity(),
            blend: c.blend(),
            color: true,
        }
    }
    fn valid(self, c: Class) -> bool {
        (!self.appearance && !self.expression || c.appearance())
            && (!self.opacity || c.opacity())
            && (!self.blend || c.blend())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub class: Class,
    pub fields: Fields,
    pub appearance: Style,
    pub expression: Expression,
    pub opacity: u8,
    pub blend: crate::layer_blend::Mode,
    pub color: Option<[u8; 3]>,
}
impl Entry {
    fn valid(&self) -> bool {
        self.class.valid()
            && self.fields.valid(self.class)
            && self.appearance.valid()
            && self.appearance.scope.is_none()
            && self.opacity <= 100
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub entries: Vec<Entry>,
}
impl Data {
    pub fn is_default(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.entries.len() <= 64
            && self.entries.iter().all(Entry::valid)
            && self
                .entries
                .iter()
                .map(|e| e.class)
                .collect::<BTreeSet<_>>()
                .len()
                == self.entries.len()
    }
    pub fn get(&self, class: Class) -> Option<&Entry> {
        self.entries.iter().find(|e| e.class == class)
    }
    pub fn set(&mut self, e: Entry) -> Result<(), String> {
        if !e.valid() {
            return Err("Unsupported or invalid layer default properties".into());
        }
        let mut next = self.clone();
        next.entries.retain(|v| v.class != e.class);
        next.entries.push(e);
        next.entries.sort_by_key(|e| e.class);
        if !next.valid() {
            return Err("Layer defaults limit".into());
        }
        *self = next;
        Ok(())
    }
    pub fn reset(&mut self, class: Class) {
        self.entries.retain(|e| e.class != class);
    }
}
pub fn class(d: &Document, key: Key) -> Result<Class, String> {
    let t = catalog::target(d, key).ok_or("Missing layer default source")?;
    let kind = catalog::rows(d)?
        .into_iter()
        .find(|r| r.key == key)
        .ok_or("Missing layer default source")?
        .kind;
    Ok(class_kind(t, kind))
}
fn class_kind(t: Target, kind: Kind) -> Class {
    let kind = match kind {
        Kind::RasterGray | Kind::RasterMono => Kind::RasterColor,
        Kind::VectorGray | Kind::VectorMono => Kind::VectorColor,
        k => k,
    };
    let storage = match t {
        Target::Raster(_) => Storage::Raster,
        Target::Vector(_) => Storage::Vector,
        Target::Folder(_) => Storage::Folder,
        Target::Track(_) => Storage::Track,
        Target::Cell { .. } => Storage::Cell,
        Target::Object(_) => Storage::Object,
        Target::Gradient(_) => Storage::Gradient,
        Target::World(_) => Storage::World,
        Target::Selection(_) => Storage::Selection,
    };
    Class { kind, storage }
}
pub fn capture(d: &Document, key: Key, fields: Fields) -> Result<Entry, String> {
    let class = class(d, key)?;
    if !fields.valid(class) {
        return Err("Those properties are unsupported for this layer kind".into());
    }
    let t = catalog::target(d, key).ok_or("Missing default source")?;
    let properties = p::properties(d, t);
    let mut appearance = d.scene.appearance.get(t).cloned().unwrap_or_default();
    appearance.scope = None;
    appearance.scope_space = Default::default();
    let e = Entry {
        class,
        fields,
        appearance: if fields.appearance {
            appearance
        } else {
            Style::default()
        },
        expression: if fields.expression {
            crate::appearance::expression(d, t)
        } else {
            Expression::Color
        },
        opacity: if fields.opacity {
            properties.opacity
        } else {
            100
        },
        blend: if fields.blend {
            properties.blend
        } else {
            Default::default()
        },
        color: if fields.color { properties.color } else { None },
    };
    if !e.valid() {
        return Err("Invalid layer defaults".into());
    }
    Ok(e)
}
fn apply(
    d: &Document,
    s: &mut DocumentState,
    data: &Data,
    existing: &BTreeSet<Key>,
) -> Result<(), String> {
    if data.is_default() {
        return Ok(());
    }
    if !data.valid() || s.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Invalid layer defaults or creation snapshot limit".into());
    }
    catalog::reconcile(d, s)?;
    let after = crate::layer_merge::preview(s, d.current_frame)?;
    let rows = catalog::rows(&after)?;
    for row in rows.into_iter().filter(|r| !existing.contains(&r.key)) {
        let t = catalog::target(&after, row.key).ok_or("Missing created layer")?;
        let class = class_kind(t, row.kind);
        let Some(e) = data.get(class) else {
            continue;
        };
        let old = p::properties(&after, t);
        let value = Properties {
            opacity: if e.fields.opacity {
                e.opacity
            } else {
                old.opacity
            },
            blend: if e.fields.blend { e.blend } else { old.blend },
            color: if e.fields.color { e.color } else { old.color },
            ..old
        };
        p::set_state(&after, s, t, value)?;
        if let Target::Folder(n) = t
            && !value.blend.is_normal()
        {
            s.scene.paper.folders[n as usize].through = false;
        }
        if e.fields.appearance {
            s.scene.appearance.set(t, e.appearance.clone())?;
        }
        if e.fields.expression {
            let mut info = s.scene.catalog.info(row.key).cloned().unwrap_or_default();
            info.expression = e.expression;
            if info.kind.is_some_and(|k| {
                matches!(
                    k,
                    Kind::RasterColor
                        | Kind::RasterGray
                        | Kind::RasterMono
                        | Kind::VectorColor
                        | Kind::VectorGray
                        | Kind::VectorMono
                )
            }) {
                info.kind = None;
            }
            s.scene.catalog.set(row.key, info);
        }
    }
    if !s.scene.valid()
        || !s.scene.paper.valid_for(s.layers.len())
        || serde_json::to_vec(&s.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Layer defaults exceed scene limits".into());
    }
    crate::appearance::limits(s.width, s.height, &s.layers, &s.scene)?;
    Ok(())
}
/// Call only for explicit creation, never duplication, merging, baking or reopening a file.
pub fn apply_new(d: &Document, s: &mut DocumentState, data: &Data) -> Result<(), String> {
    if data.is_default() {
        return Ok(());
    }
    let existing = catalog::rows(d)?.iter().map(|r| r.key).collect();
    apply(d, s, data, &existing)
}
pub fn initialize(d: &mut Document, data: &Data) -> Result<(), String> {
    if data.is_default() {
        return Ok(());
    }
    let mut s = d.snapshot();
    apply(d, &mut s, data, &BTreeSet::new())?;
    d.scene = s.scene;
    Ok(())
}
pub fn scene(
    d: &Document,
    scene: crate::scene::Scene,
    data: &Data,
) -> Result<crate::scene::Scene, String> {
    if data.is_default() {
        return Ok(scene);
    }
    let mut s = d.snapshot();
    s.scene = scene;
    apply_new(d, &mut s, data)?;
    Ok(s.scene)
}
