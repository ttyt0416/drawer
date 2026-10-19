//! Page-owned reference/draft flags; sampling and rendered output are separate views.
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET, Document},
    fill_reference::{Role, Tags},
    layer_catalog as c,
    layer_properties::{self as p, Target},
    layer_search::Key,
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub reference: bool,
    pub draft: bool,
}
pub fn is_false(v: &bool) -> bool {
    !*v
}
pub fn own(d: &Document, t: Target) -> Flags {
    let tags = match t {
        Target::Raster(id) => crate::layers::index(d, id)
            .and_then(|n| d.scene.paper.layers.get(&n))
            .map(|v| v.fill),
        Target::Vector(id) => d
            .scene
            .lines
            .layers
            .iter()
            .find(|v| v.id == id)
            .map(|v| v.fill),
        Target::Folder(n) => d.scene.paper.folders.get(n as usize).map(|v| v.paper.fill),
        _ => None,
    }
    .unwrap_or_default();
    let info = d.scene.catalog.info(c::key(d, t));
    Flags {
        reference: tags.reference || info.is_some_and(|i| i.reference),
        draft: tags.draft || info.is_some_and(|i| i.draft),
    }
}
pub fn flags(d: &Document, t: Target) -> Flags {
    let mut value = own(d, t);
    let mut node = match t {
        Target::Raster(id) => crate::layers::index(d, id).map(crate::paper::Node::Raster),
        Target::Folder(n) => Some(crate::paper::Node::Folder(n as usize)),
        _ => None,
    };
    for _ in 0..16 {
        let Some(n) = node else {
            break;
        };
        let Some(parent) = d
            .scene
            .paper
            .folders
            .iter()
            .position(|f| f.children.contains(&n))
        else {
            break;
        };
        let f = own(d, Target::Folder(parent as u16));
        value.reference |= f.reference;
        value.draft |= f.draft;
        node = Some(crate::paper::Node::Folder(parent));
    }
    if let Target::Cell { track, .. } = t {
        let f = own(d, Target::Track(track));
        value.reference |= f.reference;
        value.draft |= f.draft;
    }
    value
}
pub fn tags(d: &Document, t: Target) -> Tags {
    let mut tags = match t {
        Target::Raster(id) => crate::layers::index(d, id)
            .and_then(|n| d.scene.paper.layers.get(&n))
            .map_or(Tags::default(), |v| v.fill),
        Target::Vector(id) => d
            .scene
            .lines
            .layers
            .iter()
            .find(|v| v.id == id)
            .map_or(Tags::default(), |v| v.fill),
        _ => Tags::default(),
    };
    let f = flags(d, t);
    tags.reference = f.reference;
    tags.draft = f.draft;
    if matches!(t,Target::Object(id) if c::source_index(d,Key::Object(id)).is_some_and(|n|matches!(&d.scene.objects[n],crate::scene::PageObject::Text{..})))
    {
        tags.role = Role::Text;
    }
    if let Some(kind) = d.scene.catalog.info(c::key(d, t)).and_then(|i| i.kind) {
        match kind {
            crate::layer_search::Kind::Text => tags.role = Role::Text,
            crate::layer_search::Kind::Balloon => tags.role = Role::Balloon,
            _ => {}
        }
    }
    tags
}
pub fn prepare(
    d: &Document,
    key: Key,
    value: Flags,
    include_drafts: Option<bool>,
) -> Result<DocumentState, String> {
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 || c::thumbnail(d, key).is_none() {
        return Err("Missing role target or snapshot limit".into());
    }
    let t = c::target(d, key).ok_or("Missing role target")?;
    let mut s = d.snapshot();
    match t {
        Target::Raster(id) => {
            let n = crate::layers::index(d, id).ok_or("Missing raster")?;
            let mut paper = s.scene.paper.layers.get(&n).copied().unwrap_or_default();
            paper.fill.reference = value.reference;
            paper.fill.draft = value.draft;
            if paper == Default::default() {
                s.scene.paper.layers.remove(&n);
            } else {
                s.scene.paper.layers.insert(n, paper);
            }
        }
        Target::Vector(id) => {
            let l = s
                .scene
                .lines
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or("Missing vector")?;
            l.fill.reference = value.reference;
            l.fill.draft = value.draft;
        }
        _ => {
            let mut info = s.scene.catalog.info(key).cloned().unwrap_or_default();
            info.reference = value.reference;
            info.draft = value.draft;
            s.scene.catalog.set(key, info);
            if let Target::Folder(n) = t {
                let f = &mut s.scene.paper.folders[n as usize];
                f.paper.fill.reference = false;
                f.paper.fill.draft = false;
            }
        }
    }
    if let Some(v) = include_drafts {
        s.scene.include_drafts_in_output = v;
    }
    if matches!(t, Target::Raster(_) | Target::Vector(_)) {
        let mut info = s.scene.catalog.info(key).cloned().unwrap_or_default();
        info.reference = false;
        info.draft = false;
        s.scene.catalog.set(key, info);
    }
    validate(d, &s)?;
    Ok(s)
}
fn validate(d: &Document, s: &DocumentState) -> Result<(), String> {
    if !s.scene.valid()
        || !s.scene.paper.valid_for(s.layers.len())
        || s.memory_bytes().saturating_add(d.memory_bytes()) > DEFAULT_DOCUMENT_BUDGET / 4
        || serde_json::to_vec(&s.scene)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("Layer roles exceed scene/Undo limits".into());
    }
    p::validate_edit(d, s)
}
pub fn has_drafts(d: &Document) -> bool {
    p::targets(d)
        .into_iter()
        .filter(|t| !matches!(t, Target::Selection(_)))
        .any(|t| flags(d, t).draft)
}
pub(crate) fn common(
    d: &Document,
    indices: impl IntoIterator<Item = usize>,
) -> Result<Flags, String> {
    let mut value = None;
    for n in indices {
        let f = flags(d, p::raster(d, n));
        if value.is_some_and(|v| v != f) {
            return Err("Selected layers have different reference/draft roles; make their roles consistent before merging or baking a subtree".into());
        }
        value = Some(f);
    }
    Ok(value.unwrap_or_default())
}
pub(crate) fn put_raster(s: &mut DocumentState, n: usize, f: Flags) {
    let mut paper = s.scene.paper.layers.get(&n).copied().unwrap_or_default();
    paper.fill.reference = f.reference;
    paper.fill.draft = f.draft;
    if paper == Default::default() {
        s.scene.paper.layers.remove(&n);
    } else {
        s.scene.paper.layers.insert(n, paper);
    }
}
pub(crate) fn inherit_unwrap(d: &Document, s: &mut DocumentState, folder: usize) {
    let parent = own(d, Target::Folder(folder as u16));
    if parent == Flags::default() {
        return;
    }
    for node in &d.scene.paper.folders[folder].children {
        let t = crate::clipping::node_key(d, *node);
        let child = own(d, t);
        let f = Flags {
            reference: parent.reference || child.reference,
            draft: parent.draft || child.draft,
        };
        match node {
            crate::paper::Node::Raster(n) => put_raster(s, *n, f),
            crate::paper::Node::Folder(n) => {
                let key = Key::Layer(Target::Folder(*n as u16));
                let mut info = s.scene.catalog.info(key).cloned().unwrap_or_default();
                info.reference = f.reference;
                info.draft = f.draft;
                s.scene.catalog.set(key, info);
                s.scene.paper.folders[*n].paper.fill.reference = false;
                s.scene.paper.folders[*n].paper.fill.draft = false;
            }
        }
    }
}
pub(crate) fn output_view(d: &Document) -> Result<Option<Box<Document>>, String> {
    if d.scene.include_drafts_in_output || !has_drafts(d) {
        return Ok(None);
    }
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Draft output snapshot limit".into());
    }
    let mut s = d.snapshot();
    for t in p::targets(d) {
        if flags(d, t).draft {
            let mut v = p::properties(d, t);
            v.hidden = true;
            p::set_state(d, &mut s, t, v)?;
        }
    }
    Ok(Some(Box::new(crate::layer_merge::preview(
        &s,
        d.current_frame,
    )?)))
}
pub(crate) fn reference_view(
    d: &Document,
    active: usize,
    s: crate::bucket::Settings,
) -> Result<Box<Document>, String> {
    use crate::bucket::Reference;
    if d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Reference snapshot limit".into());
    }
    let mut state = d.snapshot();
    let accepted = |t: Target| {
        let info = tags(d, t);
        let properties = p::inherited(d, t);
        let scope = match s.reference {
            Reference::Composite => true,
            Reference::Marked => info.reference,
            Reference::Selected => match t {
                Target::Raster(id) => crate::layers::index(d, id)
                    .is_some_and(|n| crate::fill_reference::selected(s.selected, n)),
                Target::Vector(id) => d
                    .scene
                    .lines
                    .layers
                    .iter()
                    .position(|l| l.id == id)
                    .is_some_and(|n| crate::fill_reference::selected(s.selected_vectors, n)),
                _ => false,
            },
            Reference::Folder(n) => match t {
                Target::Raster(id) => crate::layers::index(d, id)
                    .is_some_and(|i| crate::fill_reference::in_folder(d, n as usize, i, 0)),
                _ => false,
            },
            Reference::Active => {
                matches!(t,Target::Raster(id) if crate::layers::index(d,id)==Some(active))
            }
            Reference::Layer(n) => {
                matches!(t,Target::Raster(id) if crate::layers::index(d,id)==Some(n as usize))
            }
        };
        scope
            && (s.include_hidden || !properties.hidden)
            && s.exclude.accepts(info, properties.locked)
            && !(s.exclude.editing
                && matches!(t,Target::Raster(id) if crate::layers::index(d,id)==Some(active)))
    };
    let targets = p::targets(d);
    let included = targets
        .iter()
        .copied()
        .map(|t| (t, accepted(t)))
        .collect::<std::collections::BTreeMap<_, _>>();
    for t in targets {
        let keep = match t {
            Target::Folder(n) => (0..d.layers.len()).any(|i| {
                crate::fill_reference::in_folder(d, n as usize, i, 0)
                    && included.get(&p::raster(d, i)) == Some(&true)
            }),
            Target::Track(n) => d.animation.as_ref().is_some_and(|a| {
                a.tracks[n as usize].cells.iter().enumerate().any(|(i, _)| {
                    included.get(&Target::Cell {
                        track: n,
                        cell: i as u16,
                    }) == Some(&true)
                })
            }),
            _ => included[&t],
        };
        let mut value = p::properties(d, t);
        value.hidden = !keep;
        p::set_state(d, &mut state, t, value)?;
    }
    if s.include_hidden {
        state.scene.clipping.solo = None;
    }
    Ok(Box::new(crate::layer_merge::preview(
        &state,
        d.current_frame,
    )?))
}
