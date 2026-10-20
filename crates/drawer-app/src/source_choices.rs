use drawer_core::file_sources::{Content, Metadata, Selection};
use eframe::egui;
pub(super) fn controls(
    ui: &mut egui::Ui,
    selection: &mut Selection,
    metadata: Option<&Metadata>,
) -> bool {
    let before = selection.clone();
    let Some(info) = metadata else {
        ui.small("Inspect the source to choose a layer, comp, timeline or frame/page. Saved choices are kept until you explicitly change them.");
        return false;
    };
    ui.label(format!(
        "{} source · {} frames/pages",
        if info.native {
            "Native"
        } else {
            "Image exchange"
        },
        info.frames
    ));
    let selected = match selection.content {
        Content::Page => "Whole page".into(),
        Content::Timeline => "Timeline only".into(),
        Content::Layer(key) => info
            .layers
            .iter()
            .find(|v| v.0 == key)
            .map_or_else(|| format!("Missing saved layer {key:?}"), |v| v.1.clone()),
    };
    ui.horizontal(|ui| {
        ui.label("Content");
        egui::ComboBox::from_id_salt("source-content")
            .selected_text(selected)
            .width(275.)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selection.content, Content::Page, "Whole page");
                if info.timeline {
                    ui.selectable_value(&mut selection.content, Content::Timeline, "Timeline only");
                }
                for (key, name) in &info.layers {
                    ui.selectable_value(&mut selection.content, Content::Layer(*key), name);
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Visibility comp");
        let text = selection.comp.map_or_else(
            || "Stored visibility".into(),
            |id| {
                info.comps
                    .iter()
                    .find(|v| v.0 == id)
                    .map_or_else(|| format!("Missing saved comp {id}"), |v| v.1.clone())
            },
        );
        egui::ComboBox::from_id_salt("source-comp")
            .selected_text(text)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selection.comp, None, "Stored visibility");
                for (id, name) in &info.comps {
                    ui.selectable_value(&mut selection.comp, Some(*id), name);
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Frame / page");
        let mut value = selection.frame.saturating_add(1);
        if ui
            .add(egui::DragValue::new(&mut value).range(1..=10000))
            .changed()
        {
            selection.frame = value - 1;
        }
        ui.small(format!("Available: 1–{}", info.frames));
    });
    ui.small("Missing choices cause a refresh error and preserve the existing cache; they never select a different layer automatically.");
    before != *selection
}
