//! Literal Unicode layer queries. Type/attachment union precedes name/state/scope intersection.
use crate::layer_properties::Target;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    Layer(Target),
    Track(u64),
    Cell { track: u64, cell: u64 },
    Object(u64),
    Gradient(u64),
    World(u64),
    Selection(u64),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Expression {
    Mono,
    Gray,
    #[default]
    Color,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    RasterColor,
    RasterGray,
    RasterMono,
    VectorColor,
    VectorGray,
    VectorMono,
    Correction,
    Gradient,
    Solid,
    Text,
    Balloon,
    Image,
    Focus,
    Speed,
    World,
    PanelFolder,
    Folder,
    AnimationFolder,
    Cell,
    Tone,
    Ruler,
    Mask,
    Selection,
    Draft,
    Broader,
}
impl Kind {
    pub const ALL: &[Self] = &[
        Self::RasterColor,
        Self::RasterGray,
        Self::RasterMono,
        Self::VectorColor,
        Self::VectorGray,
        Self::VectorMono,
        Self::Correction,
        Self::Gradient,
        Self::Solid,
        Self::Text,
        Self::Balloon,
        Self::Image,
        Self::Focus,
        Self::Speed,
        Self::World,
        Self::PanelFolder,
        Self::Folder,
        Self::AnimationFolder,
        Self::Cell,
        Self::Tone,
        Self::Ruler,
        Self::Mask,
        Self::Selection,
        Self::Draft,
        Self::Broader,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::RasterColor => "Raster / color",
            Self::RasterGray => "Raster / gray",
            Self::RasterMono => "Raster / monochrome",
            Self::VectorColor => "Vector / color",
            Self::VectorGray => "Vector / gray",
            Self::VectorMono => "Vector / monochrome",
            Self::Correction => "Correction",
            Self::Gradient => "Gradient",
            Self::Solid => "Solid fill",
            Self::Text => "Text",
            Self::Balloon => "Balloon",
            Self::Image => "Image material",
            Self::Focus => "Focus lines",
            Self::Speed => "Speed lines",
            Self::World => "3D",
            Self::PanelFolder => "Panel folder",
            Self::Folder => "Layer folder",
            Self::AnimationFolder => "Animation folder",
            Self::Cell => "Animation cell",
            Self::Tone => "Tone",
            Self::Ruler => "Ruler attached",
            Self::Mask => "Mask attached",
            Self::Selection => "Selection layer",
            Self::Draft => "Draft",
            Self::Broader => "Broader color than document",
        }
    }
    pub fn folder(self) -> bool {
        matches!(
            self,
            Self::PanelFolder | Self::Folder | Self::AnimationFolder
        )
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kinds {
    #[default]
    All,
    Some(BTreeSet<Kind>),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    #[default]
    Any,
    Include,
    Exclude,
}
impl Condition {
    pub const ALL: [Self; 3] = [Self::Any, Self::Include, Self::Exclude];
    pub fn matches(self, value: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Include => value,
            Self::Exclude => !value,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Context {
    #[default]
    Folder,
    Panel,
    Animation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    All,
    Inside(Context),
    Outside(Context),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Tree,
    Name,
    Kind,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub name: String,
    pub tag: String,
    pub kinds: Kinds,
    pub states: [Condition; 7],
    pub scope: Scope,
    pub sort: Sort,
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.name.len() <= 1024
            && self.tag.len() <= 256
            && match &self.kinds {
                Kinds::All => true,
                Kinds::Some(k) => k.len() <= Kind::ALL.len(),
            }
    }
}
pub const STATES: [&str; 7] = [
    "Visible",
    "Layer lock",
    "Alpha protection",
    "Position lock",
    "Image pixel lock",
    "Reference",
    "Draft",
];
#[derive(Clone, Debug)]
pub struct Row {
    pub key: Key,
    pub name: String,
    pub kind: Kind,
    pub expression: Expression,
    pub extras: BTreeSet<Kind>,
    pub tags: Vec<String>,
    pub states: [bool; 7],
    pub parent: Option<Key>,
}
pub fn fold(value: &str) -> String {
    icu_casemap::CaseMapper::new()
        .fold_string(&value.nfc().collect::<String>())
        .nfc()
        .collect()
}
fn root(rows: &BTreeMap<Key, &Row>, primary: Option<Key>, context: Context) -> Option<Key> {
    let mut key = primary;
    for _ in 0..64 {
        let row = rows.get(&key?)?;
        let matches = match context {
            Context::Folder => row.kind.folder(),
            Context::Panel => row.kind == Kind::PanelFolder,
            Context::Animation => row.kind == Kind::AnimationFolder,
        };
        if matches {
            return Some(row.key);
        }
        key = row.parent;
    }
    None
}
fn inside(rows: &BTreeMap<Key, &Row>, key: Key, root: Key) -> bool {
    let mut k = Some(key);
    for _ in 0..64 {
        let Some(row) = k.and_then(|k| rows.get(&k)) else {
            return false;
        };
        if row.key == root {
            return true;
        }
        k = row.parent;
    }
    false
}
pub fn query(
    rows: &[Row],
    settings: &Settings,
    primary: Option<Key>,
    document_expression: Expression,
) -> Result<Vec<Key>, String> {
    if !settings.valid() || rows.len() > 16384 {
        return Err("Layer search query/catalog limit".into());
    }
    let map = rows.iter().map(|r| (r.key, r)).collect::<BTreeMap<_, _>>();
    if map.len() != rows.len()
        || rows.iter().any(|r| {
            r.name.len() > 16384 || r.tags.len() > 16 || r.tags.iter().any(|s| s.len() > 128)
        })
    {
        return Err("Duplicate or oversized layer search records".into());
    }
    let context = match settings.scope {
        Scope::All => None,
        Scope::Inside(c) | Scope::Outside(c) => {
            let Some(root) = root(&map, primary, c) else {
                return Ok(vec![]);
            };
            Some(root)
        }
    };
    let name = fold(&settings.name);
    let tag = fold(&settings.tag);
    let mut results = vec![];
    for (i, row) in rows.iter().enumerate() {
        let type_matches = match &settings.kinds {
            Kinds::All => true,
            Kinds::Some(k) => {
                k.contains(&row.kind)
                    || row.extras.iter().any(|v| k.contains(v))
                    || k.contains(&Kind::Broader) && row.expression > document_expression
            }
        };
        if !type_matches
            || !row
                .states
                .iter()
                .zip(settings.states)
                .all(|(v, c)| c.matches(*v))
        {
            continue;
        }
        if let Some(root) = context {
            let belongs = inside(&map, row.key, root);
            if matches!(settings.scope, Scope::Inside(_)) != belongs {
                continue;
            }
        }
        let folded = fold(&row.name);
        if !folded.contains(&name)
            || !tag.is_empty() && !row.tags.iter().any(|t| fold(t).contains(&tag))
        {
            continue;
        }
        results.push((i, row, folded));
    }
    match settings.sort {
        Sort::Tree => {}
        Sort::Name => results.sort_by(|a, b| a.2.cmp(&b.2).then(a.1.key.cmp(&b.1.key))),
        Sort::Kind => results.sort_by_key(|(_, r, _)| (r.kind, r.key)),
    }
    Ok(results.into_iter().map(|(_, r, _)| r.key).collect())
}
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub keys: BTreeSet<Key>,
    pub primary: Option<Key>,
    pub anchor: Option<Key>,
}
impl Selection {
    pub fn retain(&mut self, rows: &[Row]) {
        let keys = rows.iter().map(|r| r.key).collect::<BTreeSet<_>>();
        self.keys.retain(|k| keys.contains(k));
        if self.primary.is_some_and(|k| !keys.contains(&k)) {
            self.primary = None;
        }
        if self.anchor.is_some_and(|k| !keys.contains(&k)) {
            self.anchor = None;
        }
    }
    pub fn click(&mut self, visible: &[Key], key: Key, control: bool, shift: bool) {
        let Some(end) = visible.iter().position(|k| *k == key) else {
            return;
        };
        let start = shift
            .then(|| {
                self.anchor
                    .and_then(|a| visible.iter().position(|k| *k == a))
            })
            .flatten();
        if !control {
            self.keys.clear();
        }
        if let Some(start) = start {
            self.keys
                .extend(visible[start.min(end)..=start.max(end)].iter().copied());
        } else if control && self.keys.contains(&key) {
            self.keys.remove(&key);
        } else {
            self.keys.insert(key);
        }
        if !shift || start.is_none() {
            self.anchor = Some(key);
        }
        self.primary = Some(key);
    }
    pub fn visible(&self, results: &[Key]) -> Vec<Key> {
        results
            .iter()
            .copied()
            .filter(|k| self.keys.contains(k))
            .collect()
    }
}
