//! Explicit conversion between Drawer layout generations; never runs while opening a file.
use crate::{document::Document, layer_search::Key, scene::PageObject, text_layout::Style};
pub const CURRENT: u8 = 2;
pub fn generation(object: &PageObject) -> Option<u8> {
    match object {
        PageObject::Text { layout, .. } => Some(layout.as_ref().map_or(0, |s| s.generation)),
        _ => None,
    }
}
pub fn upgrade(style: &Style, text: &str) -> Result<Style, String> {
    let mut next = style.clone();
    next.generation = CURRENT;
    next.rich.get_or_insert_with(Default::default);
    if !next.text_valid(text) {
        return Err("Conversion would split an existing reading/format range. Adjust its grapheme boundaries first; no text was changed.".into());
    }
    Ok(next)
}
pub struct Prepared {
    pub state: crate::sizing::DocumentState,
    pub changed: usize,
    pub notices: Vec<String>,
}
/// None selects all text objects on this page; Some selects stable object IDs only.
pub fn prepare(d: &Document, selected: Option<&[u64]>) -> Result<Prepared, String> {
    if d.memory_bytes() > crate::document::DEFAULT_DOCUMENT_BUDGET / 8 {
        return Err("Text conversion snapshot limit".into());
    }
    let indices: Vec<_> = d
        .scene
        .objects
        .iter()
        .enumerate()
        .filter(|(n, o)| {
            generation(o).is_some()
                && selected.is_none_or(|ids| ids.contains(&d.scene.catalog.objects.id(*n)))
        })
        .map(|(n, _)| n)
        .collect();
    if indices.is_empty()
        || selected.is_some_and(|ids| {
            ids.is_empty()
                || ids.iter().any(|id| {
                    !indices
                        .iter()
                        .any(|n| d.scene.catalog.objects.id(*n) == *id)
                })
        })
    {
        return Err("Select existing text objects, or choose all text on this page".into());
    }
    let keys: Vec<_> = indices
        .iter()
        .map(|n| Key::Object(d.scene.catalog.objects.id(*n)))
        .collect();
    crate::layer_groups::preflight(d, &keys, crate::layer_groups::Operation::Transform)?;
    let mut state = d.snapshot();
    let mut changed = 0;
    let mut notices = vec![];
    for n in indices {
        let PageObject::Text {
            baseline,
            text,
            size_px,
            layout,
            ..
        } = &mut state.scene.objects[n]
        else {
            unreachable!()
        };
        let before = layout.as_deref().cloned().unwrap_or_else(Style::legacy);
        if before.generation == CURRENT {
            continue;
        }
        let next = upgrade(&before, text)?;
        let shape = crate::text_layout::layout(*baseline, text, *size_px, &next)?;
        notices.push(format!(
            "Text #{}: Drawer layout {} → {}; {}",
            d.scene.catalog.objects.id(n),
            before.generation,
            CURRENT,
            shape.notice(next.font)
        ));
        *layout = Some(Box::new(next));
        changed += 1;
    }
    crate::text_layout::validate_scene(&state.scene)?;
    crate::layer_groups::finish(d, &state)?;
    Ok(Prepared {
        state,
        changed,
        notices,
    })
}
