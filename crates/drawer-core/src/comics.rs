//! Local comic composition and reference-owned frame folders/masks.
pub use crate::comic_object::*;
use crate::{
    document::Document,
    layer_properties::Target,
    layer_search::Key,
    scene::{Affine, PageObject, Point, Rect, Scene},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub target: Key,
    pub polygon: Vec<Point>,
    pub space: Affine,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PageInfo {
    pub title: String,
    pub author: String,
    pub language: String,
    pub trim_mm: [f64; 2],
    pub bleed_mm: f64,
    pub safe_mm: f64,
    pub right_binding: bool,
    pub first_right: bool,
    pub number: u32,
    pub total: u32,
    pub include_guides: bool,
    pub include_number: bool,
}
impl Default for PageInfo {
    fn default() -> Self {
        Self {
            title: String::new(),
            author: String::new(),
            language: "ko".into(),
            trim_mm: [0.; 2],
            bleed_mm: 0.,
            safe_mm: 0.,
            right_binding: false,
            first_right: true,
            number: 1,
            total: 1,
            include_guides: false,
            include_number: false,
        }
    }
}
impl PageInfo {
    pub fn valid(&self) -> bool {
        self.title.chars().count() <= 256
            && self.author.chars().count() <= 256
            && self.language.len() <= 32
            && self.trim_mm.iter().all(|v| (0. ..=1e4).contains(v))
            && (0. ..=1000.).contains(&self.bleed_mm)
            && (0. ..=1000.).contains(&self.safe_mm)
            && self.number > 0
            && self.number <= self.total
            && self.total <= 100000
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Webtoon {
    pub slice: u32,
    pub cuts: Vec<u32>,
    pub prefix: String,
}
impl Default for Webtoon {
    fn default() -> Self {
        Self {
            slice: 1280,
            cuts: vec![],
            prefix: "page".into(),
        }
    }
}
impl Webtoon {
    pub fn valid(&self) -> bool {
        self.slice > 0
            && self.slice <= 32768
            && self.cuts.len() <= 1024
            && self.cuts.windows(2).all(|r| r[0] < r[1])
            && self.cuts.iter().all(|v| *v > 0 && *v < 32768)
            && !self.prefix.is_empty()
            && self.prefix.len() <= 64
            && self
                .prefix
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Data {
    pub masks: Vec<Mask>,
    pub page: PageInfo,
    pub webtoon: Webtoon,
}
impl Data {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.masks.len() <= 1024
            && self.masks.iter().all(|m| {
                m.polygon.len() >= 3
                    && m.polygon.len() <= 128
                    && m.polygon
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite() && v.abs() <= 1e9)
                    && m.space.valid()
            })
            && self.page.valid()
            && self.webtoon.valid()
    }
    pub fn bytes(&self) -> usize {
        if self.is_default() {
            return 0;
        }
        self.masks
            .iter()
            .map(|m| 128 + m.polygon.len() * 16)
            .sum::<usize>()
            + self.page.title.len()
            + self.page.author.len()
            + self.webtoon.cuts.len() * 4
            + 512
    }
}
pub fn valid_scene(s: &Scene) -> bool {
    let mut effect_count = 0;
    let mut linked_characters = 0usize;
    for (n, o) in s.objects.iter().enumerate() {
        if let PageObject::Comic(o) = o {
            if !o.valid() {
                return false;
            }
            match &o.kind {
                Kind::Balloon(b) => {
                    if let Some(id) = b.text
                        && let Some(i) = s.catalog.objects.index(id, s.objects.len())
                        && let PageObject::Text { text, .. } = &s.objects[i]
                    {
                        linked_characters = linked_characters.saturating_add(text.chars().count());
                    }
                    if b.text.is_some_and(|id| {
                        s.catalog
                            .objects
                            .index(id, s.objects.len())
                            .is_none_or(|i| !matches!(s.objects[i], PageObject::Text { .. }))
                    }) {
                        return false;
                    }
                }
                Kind::Frame(f) => {
                    if f.children
                        .iter()
                        .any(|k| *k == Key::Object(s.catalog.objects.id(n)))
                    {
                        return false;
                    }
                    if depth(s, n, &mut vec![]) > 16 {
                        return false;
                    }
                }
                Kind::Effect(e) => effect_count += e.count as usize,
                _ => {}
            }
        }
    }
    effect_count <= 8192 && linked_characters <= 65536 && s.comics.valid()
}
fn depth(s: &Scene, n: usize, seen: &mut Vec<usize>) -> usize {
    if seen.contains(&n) {
        return 100;
    }
    seen.push(n);
    let d = if let Some(PageObject::Comic(o)) = s.objects.get(n)
        && let Kind::Frame(f) = &o.kind
    {
        f.children
            .iter()
            .filter_map(|k| {
                if let Key::Object(id) = k {
                    s.catalog.objects.index(*id, s.objects.len())
                } else {
                    None
                }
            })
            .map(|i| 1 + depth(s, i, seen))
            .max()
            .unwrap_or(1)
    } else {
        0
    };
    seen.pop();
    d
}
fn frame_has(d: &Document, n: usize, key: Key, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    let PageObject::Comic(o) = &d.scene.objects[n] else {
        return false;
    };
    let Kind::Frame(f) = &o.kind else {
        return false;
    };
    f.children.iter().any(|k| {
        *k == key
            || paper_inside(d, *k, key)
            || if let Key::Object(id) = k {
                d.scene
                    .catalog
                    .objects
                    .index(*id, d.scene.objects.len())
                    .is_some_and(|i| frame_has(d, i, key, depth + 1))
            } else {
                false
            }
    })
}
fn paper_inside(d: &Document, parent: Key, key: Key) -> bool {
    if let (Key::Track(a), Key::Cell { track: b, .. }) = (parent, key) {
        return a == b;
    }
    if let Key::Layer(Target::Folder(n)) = parent
        && let Key::Layer(Target::Raster(id)) = key
    {
        return crate::layers::index(d, id).is_some_and(|i| {
            d.scene
                .paper
                .raster_indices(crate::paper::Node::Folder(n as usize))
                .contains(&i)
        });
    }
    false
}
pub fn frame_contains(d: &Document, parent: Key, key: Key) -> bool {
    if let Key::Object(id) = parent
        && let Some(n) = d.scene.catalog.objects.index(id, d.scene.objects.len())
    {
        frame_has(d, n, key, 0)
    } else {
        false
    }
}
fn source_mask(d: &Document, key: Key, p: Point) -> bool {
    d.scene
        .comics
        .masks
        .iter()
        .all(|m| m.target != key || contains(&m.polygon, m.space.inverse().map(p)))
}
pub fn coverage(d: &Document, key: Key, p: Point) -> f64 {
    if !source_mask(d, key, p) {
        return 0.;
    }
    let mut families = std::collections::BTreeMap::<u64, bool>::new();
    for (n, o) in d.scene.objects.iter().enumerate() {
        if let PageObject::Comic(o) = o
            && let Kind::Frame(f) = &o.kind
            && frame_has(d, n, key, 0)
        {
            let id = d.scene.catalog.objects.id(n);
            let family = if f.family == 0 { id } else { f.family };
            let local = o.transform.inverse().map(p);
            let visible = !crate::layer_properties::inherited(d, Target::Object(id)).hidden;
            let inside = visible
                && source_mask(d, Key::Object(id), p)
                && crate::frame_ops::contains(f, local)
                && crate::frame_ops::edge_distance(f, local) + 1e-9 >= f.gap / 2.;
            *families.entry(family).or_default() |= inside;
        }
    }
    if families.values().any(|v| !*v) {
        0.
    } else {
        1.
    }
}
pub fn shift_masks(s: &mut Scene, key: Key, matrix: Affine) {
    for mask in &mut s.comics.masks {
        if mask.target == key {
            mask.space = mask.space.then(matrix);
        }
    }
}
pub fn prepare(d: &Document, id: Option<u64>, mut object: Object) -> Result<DocumentState, String> {
    if let Kind::Frame(f) = &mut object.kind {
        crate::frame_ops::refresh(f)?;
    }
    if let Some(id) = id {
        crate::layer_groups::preflight(
            d,
            &[Key::Object(id)],
            crate::layer_groups::Operation::Transform,
        )?;
    }
    if let Kind::Frame(f) = &object.kind {
        let live = crate::layer_groups::live(d)?;
        if f.children.iter().any(|k| !live.contains_key(k)) {
            return Err("A frame content source was removed".into());
        }
    }
    if !object.valid() {
        return Err("Invalid comic parameters".into());
    }
    let mut state = d.snapshot();
    let old = state.scene.objects.len();
    state.scene.catalog.objects.initialize(old)?;
    if let Some(id) = id {
        let n = state
            .scene
            .catalog
            .objects
            .index(id, old)
            .ok_or("Comic object removed")?;
        if !matches!(state.scene.objects[n], PageObject::Comic(_)) {
            return Err("Select a comic object".into());
        }
        state.scene.objects[n] = PageObject::Comic(Box::new(object));
    } else {
        state
            .scene
            .objects
            .push(PageObject::Comic(Box::new(object)));
        state.scene.catalog.objects.append(old + 1)?;
    }
    crate::layer_catalog::reconcile(d, &mut state)?;
    crate::render_order::reconcile(&mut state);
    crate::layer_groups::finish(d, &state)?;
    Ok(state)
}
pub fn defaults(rect: Rect) -> [Object; 4] {
    let center = [
        (rect.min[0] + rect.max[0]) / 2.,
        (rect.min[1] + rect.max[1]) / 2.,
    ];
    [
        Object {
            name: "Balloon".into(),
            transform: Default::default(),
            kind: Kind::Balloon(Balloon {
                rect,
                shape: Default::default(),
                tail: Default::default(),
                tip: [center[0], rect.max[1] + 24.],
                attachment: 90.,
                tail_width: 16.,
                fill: [255; 4],
                ink: [0, 0, 0, 255],
                width: 2.,
                text: None,
                padding: 8.,
            }),
        },
        Object {
            name: "Frame folder".into(),
            transform: Default::default(),
            kind: Kind::Frame(Frame {
                component_masks: vec![],
                compound: vec![],
                outline: vec![],
                hidden_edges: vec![],
                polygon: vec![
                    rect.min,
                    [rect.max[0], rect.min[1]],
                    rect.max,
                    [rect.min[0], rect.max[1]],
                ],
                gap: 0.,
                width: 2.,
                ink: [0, 0, 0, 255],
                children: vec![],
                family: 0,
            }),
        },
        Object {
            name: "Tone".into(),
            transform: Default::default(),
            kind: Kind::Tone(Tone {
                rect,
                dot: Default::default(),
                lpi: 60.,
                density: 0.3,
                end_density: 0.8,
                gradient: false,
                angle: 45.,
                phase: [0.; 2],
                color: [0, 0, 0, 255],
                monochrome: true,
                seed: 1,
            }),
        },
        Object {
            name: "Effect lines".into(),
            transform: Default::default(),
            kind: Kind::Effect(Effect {
                rect,
                kind: Default::default(),
                center,
                angle: 0.,
                inner: 8.,
                count: 64,
                min_width: 0.3,
                max_width: 2.,
                seed: 1,
                ink: [0, 0, 0, 255],
            }),
        },
    ]
}

pub(crate) fn prune(state: &mut DocumentState) {
    let mut candidates: Vec<Key> = state.scene.comics.masks.iter().map(|m| m.target).collect();
    for object in &state.scene.objects {
        if let PageObject::Comic(c) = object
            && let Kind::Frame(f) = &c.kind
        {
            candidates.extend(&f.children);
        }
    }
    let live: std::collections::BTreeSet<_> = candidates
        .into_iter()
        .filter(|k| crate::layer_catalog::state_target(state, *k).is_some())
        .collect();
    let text_ids: std::collections::BTreeSet<_> = state
        .scene
        .objects
        .iter()
        .enumerate()
        .filter_map(|(n, o)| {
            matches!(o, PageObject::Text { .. }).then_some(state.scene.catalog.objects.id(n))
        })
        .collect();
    state
        .scene
        .comics
        .masks
        .retain(|m| live.contains(&m.target));
    for o in &mut state.scene.objects {
        if let PageObject::Comic(c) = o {
            match &mut c.kind {
                Kind::Balloon(b) => {
                    if b.text.is_some_and(|id| !text_ids.contains(&id)) {
                        b.text = None;
                    }
                }
                Kind::Frame(f) => f.children.retain(|k| live.contains(k)),
                _ => {}
            }
        }
    }
}

pub fn has_clips(s: &Scene) -> bool {
    !s.comics.masks.is_empty()||s.objects.iter().any(|o|matches!(o,PageObject::Comic(c)if matches!(&c.kind,Kind::Frame(f)if !f.children.is_empty())))
}
