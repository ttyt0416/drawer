//! Content-dependent editing losses, separate from ordinary codec/output information.
use crate::{
    document::Document,
    layer_search::{Key, Kind},
};
#[derive(Clone, Debug, Default)]
pub(crate) struct Source {
    structure: bool,
    text: usize,
    animation: bool,
    hidden_rgb: bool,
}
impl Source {
    pub fn new(d: &Document) -> Result<Self, String> {
        let rows = crate::layer_catalog::rows(d)?;
        let rows = rows
            .iter()
            .filter(|r| !matches!(r.key, Key::Selection(_)))
            .collect::<Vec<_>>();
        let structure = rows.len() != 1
            || rows.iter().any(|r| {
                !matches!(
                    r.kind,
                    Kind::RasterColor | Kind::RasterGray | Kind::RasterMono
                )
            })
            || crate::layer_properties::targets(d).into_iter().any(|t| {
                crate::layer_masks::get(d, t).is_some()
                    || crate::layer_properties::properties(d, t) != Default::default()
            })
            || d.layers.iter().any(|l| l.placement.is_some())
            || !d.scene.appearance.is_default()
            || !d.scene.file_objects.entries.is_empty()
            || d.scene.layer_comps != Default::default();
        Ok(Self {
            structure,
            text: rows.iter().filter(|r| r.kind == Kind::Text).count(),
            animation: d.animation.as_ref().is_some_and(|a| a.frames > 1),
            hidden_rgb: d
                .layers
                .iter()
                .any(|l| l.pixels.iter().any(|p| p[3] == 0 && p[..3] != [0; 3])),
        })
    }
    pub fn losses(&self) -> Vec<String> {
        let mut v = vec![];
        if self.structure {
            v.push("Layer flattening: separate layers, hidden content and object/mask/effect editing are absent from this single composite; retain the .drawer source".into());
        }
        if self.text > 0 {
            v.push(format!("Text: {} editable text objects become pixels; text/font/ruby editing remains only in the source",self.text));
        }
        if self.animation {
            v.push("Animation: only the current rendered frame is exported; timeline/cells/audio remain in the source".into());
        }
        if self.hidden_rgb {
            v.push("Alpha: invisible RGB beneath fully transparent source pixels is normalized in the composite".into());
        }
        v
    }
}
#[path = "export_loss_verify.rs"]
pub mod verify;
