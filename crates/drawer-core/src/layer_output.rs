//! Independent layer/comp output plans; live visibility and Undo are never touched.
use crate::{
    document::Document,
    layer_catalog as catalog, layer_properties as p,
    layer_search::Key,
    output_batch::{self, Plan, Settings},
};
use std::{collections::BTreeSet, path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Composite,
    Layers,
    Comps,
}
#[derive(Clone)]
struct Variant {
    key: Option<Key>,
    comp: Option<u64>,
    name: String,
}
fn variants(d: &Document, mode: Mode, hidden: bool) -> Result<Vec<Variant>, String> {
    let list = match mode {
        Mode::Composite => vec![Variant {
            key: None,
            comp: None,
            name: d.title.clone(),
        }],
        Mode::Comps => d
            .scene
            .layer_comps
            .comps
            .iter()
            .map(|v| Variant {
                key: None,
                comp: Some(v.id),
                name: v.name.clone(),
            })
            .collect(),
        Mode::Layers => catalog::rows(d)?
            .into_iter()
            .filter(|r| !matches!(r.key, Key::Selection(_)) && (hidden || r.states[0]))
            .map(|r| Variant {
                key: Some(r.key),
                comp: None,
                name: r.name,
            })
            .collect(),
    };
    if list.is_empty() || list.len() > 128 {
        return Err("Choose1..128 layer/comp outputs".into());
    }
    Ok(list)
}
fn isolate(d: &Document, key: Key) -> Result<Document, String> {
    let rows = catalog::rows(d)?;
    let selected = rows
        .iter()
        .find(|r| r.key == key)
        .ok_or("Layer disappeared")?;
    let mut keep = BTreeSet::from([key]);
    // A folder exports its complete subtree; ancestors are opened only as containers.
    loop {
        let n = keep.len();
        for row in &rows {
            if row.parent.is_some_and(|p| keep.contains(&p)) {
                keep.insert(row.key);
            }
        }
        if keep.len() == n {
            break;
        }
    }
    let mut parents = BTreeSet::new();
    let mut parent = selected.parent;
    while let Some(id) = parent {
        if !parents.insert(id) {
            return Err("Cyclic layer hierarchy".into());
        }
        parent = rows.iter().find(|r| r.key == id).and_then(|r| r.parent);
    }
    let mut state = d.snapshot();
    state.scene.include_drafts_in_output = true;
    state.scene.clipping.solo = None;
    state.scene.layer_groups.solo = None;
    for row in &rows {
        let Some(t) = catalog::target(d, row.key) else {
            continue;
        };
        let mut props = p::properties(d, t);
        if row.key == key || parents.contains(&row.key) {
            props.hidden = false;
        } else if !keep.contains(&row.key) {
            props.hidden = true;
        }
        p::set_state(d, &mut state, t, props)?;
    }
    // An isolated source has no lower backdrop; release only bindings whose base is absent.
    let targets = p::targets(d);
    for t in targets {
        if state.scene.clipping.binding(t).is_some_and(|b| {
            !keep.contains(&catalog::key(d, b)) && !parents.contains(&catalog::key(d, b))
        }) {
            state.scene.clipping.bind(t, None);
        }
    }
    crate::layer_merge::preview(&state, d.current_frame)
}
pub fn prepare(
    docs: &[Document],
    folder: &Path,
    s: &Settings,
    mode: Mode,
    hidden: bool,
    budget: usize,
    c: &AtomicBool,
) -> Result<Plan, String> {
    if mode == Mode::Composite {
        return output_batch::prepare(docs, folder, s, budget, c);
    }
    if docs.is_empty() || docs.len() > 32 {
        return Err("Choose1..32 input documents".into());
    }
    let mut settings = s.clone();
    settings.watermark = crate::output_watermark::frozen(&s.watermark, budget, c)?;
    let mut result = Plan::empty();
    let mut names = BTreeSet::new();
    let mut used = 0usize;
    let mut index = 0;
    for d in docs {
        for v in variants(d, mode, hidden)? {
            crate::asset_catalog::canceled(c)?;
            index += 1;
            if index > 128 {
                return Err("Layer/comp output limit128".into());
            }
            let (mut copy, missing) = if let Some(id) = v.comp {
                let p = crate::layer_comps::prepare(d, crate::layer_comps::Command::Apply(id))?;
                (
                    crate::layer_merge::preview(&p.state, d.current_frame)?,
                    p.missing,
                )
            } else {
                (isolate(d, v.key.unwrap())?, vec![])
            };
            copy.id = d.id;
            copy.revision = d.revision;
            copy.title = d.title.clone();
            copy.path = d.path.clone();
            copy.source_path = d.source_path.clone();
            if v.name.contains(['{', '}']) {
                return Err(
                    "Rename layer/comp names containing filename placeholders before output".into(),
                );
            }
            let mut item = settings.clone();
            item.pattern = item
                .pattern
                .replace("{layer}", &v.name)
                .replace("{comp}", &v.name)
                .replace("{index}", &format!("{index:04}"));
            item.watermark.first_is_cover = s.watermark.first_is_cover && index == 1;
            let mut plan =
                output_batch::prepare(&[copy], folder, &item, budget.saturating_sub(used), c)?;
            for f in &mut plan.files {
                if !names.insert(f.path.to_string_lossy().to_lowercase()) {
                    return Err(
                        "Layer/comp output names collide; add {index}, {layer} or {comp}".into(),
                    );
                }
                used = used.saturating_add(f.bytes.len());
                if used > 64 * 1024 * 1024 {
                    return Err("Layer/comp batch exceeds64MiB".into());
                }
                f.notices.push(format!(
                    "{}: {}; missing saved layers: {}",
                    if mode == Mode::Comps {
                        "Comp"
                    } else {
                        "Isolated layer"
                    },
                    v.name,
                    missing.len()
                ));
                for key in &missing {
                    f.notices.push(format!("Missing comp layer: {key:?}"));
                }
                if mode == Mode::Layers {
                    f.notices.push("Independent source/subtree output: no lower backdrop; clipping to an excluded base is released only in this copy. Blend modes may differ from the full composition.".into());
                }
            }
            result.append(plan);
        }
    }
    Ok(result)
}
#[path = "layer_output_verify.rs"]
pub mod verify;
