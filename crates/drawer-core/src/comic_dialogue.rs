//! Stable object IDs distinguish duplicate names. A stale row invalidates the whole edit.
use crate::{
    document::Document,
    story_text::{Page, Prepared, Query, Transaction},
    text_layout::Settings,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub page: usize,
    pub id: u64,
    pub before: Settings,
    pub after: Settings,
}
pub fn rows(documents: &[&Document]) -> Vec<Row> {
    documents
        .iter()
        .enumerate()
        .flat_map(|(page, d)| {
            d.scene
                .objects
                .iter()
                .enumerate()
                .filter_map(move |(n, o)| {
                    Settings::from_object(o).map(|s| Row {
                        page,
                        id: d.scene.catalog.objects.id(n),
                        before: s.clone(),
                        after: s,
                    })
                })
        })
        .collect()
}
pub fn prepare(documents: &[&Document], rows: &[Row]) -> Result<Prepared, String> {
    if documents.is_empty()
        || documents.len() > 128
        || rows.len() > 4096
        || documents
            .iter()
            .enumerate()
            .any(|(n, d)| documents[..n].iter().any(|a| a.id == d.id))
    {
        return Err("Invalid dialogue page/row count".into());
    }
    let group = crate::story_text::next_group();
    let mut pages = vec![];
    let mut members = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        if row.page >= documents.len() || !seen.insert((row.page, row.id)) {
            return Err("Duplicate or missing dialogue page/object".into());
        }
    }
    for (page, d) in documents.iter().enumerate() {
        let mut state = d.snapshot();
        for row in rows.iter().filter(|r| r.page == page) {
            let n = state
                .scene
                .catalog
                .objects
                .index(row.id, state.scene.objects.len())
                .ok_or("Dialogue object removed; reload the list")?;
            if Settings::from_object(&state.scene.objects[n]).as_ref() != Some(&row.before) {
                return Err("Dialogue source changed; reload before applying".into());
            }
            let current = crate::layer_merge::preview(&state, d.current_frame)?;
            state = crate::text_layout::prepare(&current, Some(row.id), &row.after)?.state;
        }
        if state == d.snapshot() {
            continue;
        }
        crate::text_layout::validate_scene(&state.scene)?;
        crate::layer_groups::finish(d, &state)?;
        let mut after = (*d).clone();
        after.commit_state_group(state, Some(group))?;
        members.push((d.id, d.history_token(), after.history_token()));
        pages.push(Page {
            id: d.id,
            revision: d.revision,
            frame: d.current_frame,
            token: d.history_token(),
            after,
        });
    }
    Ok(Prepared {
        query: Query {
            find: String::new(),
            replace: String::new(),
            case_sensitive: true,
        },
        pages,
        matches: vec![],
        transaction: Transaction::new(group, members),
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Insertion {
    Current,
    New,
}
pub fn insert(
    d: &Document,
    source: Option<u64>,
    tool: &Settings,
    text: &str,
    policy: Insertion,
    inherit_current: bool,
) -> Result<crate::text_layout::Prepared, String> {
    let current = source
        .and_then(|id| d.scene.catalog.objects.index(id, d.scene.objects.len()))
        .and_then(|n| Settings::from_object(&d.scene.objects[n]));
    if (inherit_current || policy == Insertion::Current) && current.is_none() {
        return Err("Select an existing text to append or inherit its format".into());
    }
    let mut settings = if inherit_current {
        current.clone().unwrap()
    } else {
        tool.clone()
    };
    let prior_text = settings.text.clone();
    if policy == Insertion::Current {
        let old = current.unwrap();
        settings.baseline = old.baseline;
        settings.text = old.text;
        settings.text.push_str(text);
    } else {
        settings.text = text.into();
    }
    settings.style.remap_text(&prior_text, &settings.text);
    crate::text_layout::prepare(
        d,
        if policy == Insertion::Current {
            source
        } else {
            None
        },
        &settings,
    )
}
