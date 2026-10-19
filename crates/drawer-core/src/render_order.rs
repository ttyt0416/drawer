//! Editable raster substitutes retain the compositing position of the source they replace.
use crate::{
    document::Document, layer_properties::Target, layer_search::Key, scene::Scene,
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Group {
    Gradient,
    Object,
    Vector,
    World,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub key: Key,
    pub group: Group,
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
        self.entries.len() <= 8192
            && self.entries.iter().all(|e| match e.key {
                Key::Layer(Target::Raster(n)) => n > 0,
                Key::Layer(Target::Vector(n)) => n > 0 && e.group == Group::Vector,
                Key::Object(n) => n > 0 && e.group == Group::Object,
                Key::Gradient(n) => n > 0 && e.group == Group::Gradient,
                Key::World(n) => n > 0 && e.group == Group::World,
                _ => false,
            })
            && self
                .entries
                .iter()
                .map(|e| e.key)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.entries.len()
    }
    pub fn deferred(&self, id: u64) -> bool {
        self.entries
            .iter()
            .any(|e| e.key == Key::Layer(Target::Raster(id)))
    }
}
pub fn sources(s: &Scene) -> Vec<Entry> {
    s.gradients
        .layers
        .iter()
        .map(|g| Entry {
            key: Key::Gradient(g.id),
            group: Group::Gradient,
        })
        .chain((0..s.objects.len()).map(|n| Entry {
            key: Key::Object(s.catalog.objects.id(n)),
            group: Group::Object,
        }))
        .chain(s.lines.layers.iter().map(|l| Entry {
            key: Key::Layer(Target::Vector(l.id)),
            group: Group::Vector,
        }))
        .chain((0..s.worlds.len()).map(|n| Entry {
            key: Key::World(s.catalog.worlds.id(n)),
            group: Group::World,
        }))
        .collect()
}
pub fn normalized(s: &Scene) -> Vec<Entry> {
    let sources = sources(s);
    if s.render_order.is_default() {
        return sources;
    }
    let keys = sources
        .iter()
        .map(|e| e.key)
        .collect::<std::collections::BTreeSet<_>>();
    let mut entries = s
        .render_order
        .entries
        .iter()
        .copied()
        .filter(|e| matches!(e.key, Key::Layer(Target::Raster(_))) || keys.contains(&e.key))
        .collect::<Vec<_>>();
    for e in sources {
        if !entries.iter().any(|old| old.key == e.key) {
            let at = entries
                .iter()
                .rposition(|old| old.group == e.group)
                .map(|n| n + 1)
                .unwrap_or_else(|| {
                    entries
                        .iter()
                        .position(|old| old.group > e.group)
                        .unwrap_or(entries.len())
                });
            entries.insert(at, e);
        }
    }
    entries
}
pub(crate) fn reconcile_scene(s: &mut Scene) {
    if !s.render_order.is_default() {
        s.render_order.entries = normalized(s);
    }
}
pub(crate) fn relationships_valid(s: &Scene) -> bool {
    if s.render_order.is_default() {
        return true;
    }
    if !s
        .render_order
        .entries
        .windows(2)
        .all(|e| e[0].group <= e[1].group)
    {
        return false;
    }
    for (owner, base) in &s.clipping.vectors {
        let Target::Vector(base) = base else {
            continue;
        };
        let Some(a) = s.lines.layers.iter().position(|l| l.id == *base) else {
            continue;
        };
        let Some(b) = s.lines.layers.iter().position(|l| l.id == *owner) else {
            continue;
        };
        if a >= b
            || s.clipping.binding(Target::Vector(*base)).is_some()
            || s.lines.layers[a + 1..=b]
                .iter()
                .any(|l| s.clipping.binding(Target::Vector(l.id)).is_none())
        {
            continue;
        }
        let Some(x) = s
            .render_order
            .entries
            .iter()
            .position(|e| e.key == Key::Layer(Target::Vector(*base)))
        else {
            continue;
        };
        let Some(y) = s
            .render_order
            .entries
            .iter()
            .position(|e| e.key == Key::Layer(Target::Vector(*owner)))
        else {
            continue;
        };
        if x >= y
            || s.render_order.entries[x..=y]
                .iter()
                .any(|e| !matches!(e.key, Key::Layer(Target::Vector(_))))
        {
            return false;
        }
    }
    true
}
pub(crate) fn reconcile(s: &mut DocumentState) {
    reconcile_scene(&mut s.scene);
    let ids = (0..s.layers.len())
        .map(|n| s.scene.layer_ids.id(n))
        .collect::<std::collections::BTreeSet<_>>();
    let parented = s
        .scene
        .paper
        .folders
        .iter()
        .flat_map(|f| &f.children)
        .filter_map(|n| {
            if let crate::paper::Node::Raster(n) = n {
                Some(s.scene.layer_ids.id(*n))
            } else {
                None
            }
        })
        .collect::<std::collections::BTreeSet<_>>();
    s.scene.render_order.entries.retain(|e|!matches!(e.key,Key::Layer(Target::Raster(id))if !ids.contains(&id)||parented.contains(&id)));
}
pub(crate) fn valid_live(d: &Document) -> bool {
    let sources = sources(&d.scene)
        .into_iter()
        .map(|e| e.key)
        .collect::<std::collections::BTreeSet<_>>();
    if d.scene.clipping.rasters.iter().any(|(id, base)| {
        d.scene.render_order.deferred(*id)
            || matches!(base,Target::Raster(id)if d.scene.render_order.deferred(*id))
    }) {
        return false;
    }
    d.scene.render_order.entries.iter().all(|e| match e.key {
        Key::Layer(Target::Raster(id)) => crate::layers::index(d, id).is_some_and(|n| {
            !d.scene
                .paper
                .folders
                .iter()
                .any(|f| f.children.contains(&crate::paper::Node::Raster(n)))
        }),
        _ => sources.contains(&e.key),
    })
}
pub(crate) fn duplicate(s: &mut Scene, old: u64, new: u64) {
    if let Some(n) = s
        .render_order
        .entries
        .iter()
        .position(|e| e.key == Key::Layer(Target::Raster(old)))
    {
        let mut e = s.render_order.entries[n];
        e.key = Key::Layer(Target::Raster(new));
        s.render_order.entries.insert(n + 1, e);
    }
}
pub(crate) fn move_vector(s: &mut Scene) {
    if s.render_order.is_default() {
        return;
    }
    let mut ids = s.lines.layers.iter().map(|l| l.id);
    for e in &mut s.render_order.entries {
        if matches!(e.key, Key::Layer(Target::Vector(_)))
            && let Some(id) = ids.next()
        {
            e.key = Key::Layer(Target::Vector(id));
        }
    }
}
pub(crate) fn allow_partial_merge(d: &Document, targets: &[Target]) -> Result<(), String> {
    if targets
        .iter()
        .any(|t| matches!(t,Target::Raster(id)if d.scene.render_order.deferred(*id)))
    {
        Err("Move the raster substitute into the normal raster stack, or use Flatten page to preserve its source position".into())
    } else {
        Ok(())
    }
}
