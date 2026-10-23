use super::*;
use drawer_core::quick_filters::{Input, Kind, Mode, Output, Presets, Recipe};
pub(super) enum PresetAction {
    Save(String, Recipe),
    Delete(String),
}
pub(super) fn controls(
    ui: &mut egui::Ui,
    e: &mut Draft,
    d: &Document,
    presets: &Presets,
) -> Option<PresetAction> {
    let mut action = None;
    let mut route = e.config.route.unwrap();
    let mut target = e
        .config
        .targets
        .first()
        .copied()
        .unwrap_or(Target::Raster(1));
    ui.horizontal(|ui| {
        ui.radio_value(&mut route.mode, Mode::Quick, "Quick preset → new result");
        ui.radio_value(
            &mut route.mode,
            Mode::Pixels,
            "Pixel filter → choose output",
        );
    });
    ui.horizontal(|ui| {
        if ui
            .radio_value(&mut route.input, Input::Current, "Current layer input")
            .changed()
        {
            route.kind = drawer_core::quick_filters::kind(target).unwrap_or_default();
        }
        if ui
            .radio_value(
                &mut route.input,
                Input::Composite,
                "Entire visible composite input",
            )
            .changed()
        {
            route.kind = Kind::Composite;
        }
    });
    if route.input == Input::Current {
        egui::ComboBox::from_id_salt("quick-source-target")
            .selected_text(crate::layer_properties::label(d, target))
            .show_ui(ui, |ui| {
                for t in properties::targets(d)
                    .into_iter()
                    .filter(|t| drawer_core::quick_filters::kind(*t).is_some())
                {
                    if ui
                        .selectable_value(&mut target, t, crate::layer_properties::label(d, t))
                        .changed()
                    {
                        route.kind = drawer_core::quick_filters::kind(t).unwrap();
                    }
                }
            });
        ui.horizontal(|ui| {
            ui.label(format!("Preset target type: {:?}", route.kind));
            if drawer_core::quick_filters::kind(target) != Some(route.kind)
                && ui.button("Use selected target type").clicked()
            {
                route.kind = drawer_core::quick_filters::kind(target).unwrap_or_default();
            }
        });
    }
    if route.mode == Mode::Pixels && route.input == Input::Current {
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut route.output,
                Output::Current,
                "Apply to current pixels",
            );
            ui.radio_value(
                &mut route.output,
                Output::NewLayer,
                "Create new result layer",
            );
        });
    } else {
        route.output = Output::NewLayer;
        ui.label("Output: new Normal / 100% raster layer; original sources stay editable");
    }
    e.config.route = Some(route);
    e.config.targets = vec![target];
    ui.collapsing("Quick / route presets", |ui| {
        egui::ComboBox::from_id_salt("quick-presets")
            .selected_text("Load factory or local preset")
            .show_ui(ui, |ui| {
                for (name, recipe) in drawer_core::quick_filters::factory(
                    drawer_core::quick_filters::kind(target).unwrap_or_default(),
                )
                .into_iter()
                .chain(presets.values.clone())
                {
                    if ui.selectable_label(false, &name).clicked() {
                        e.config.task = recipe.task;
                        e.config.route = Some(recipe.route);
                        e.preset_name = name;
                        e.controls = Default::default();
                    }
                }
            });
        ui.add(egui::TextEdit::singleline(&mut e.preset_name).char_limit(64));
        ui.horizontal(|ui| {
            if ui.button("Save route preset locally").clicked() {
                action = Some(PresetAction::Save(
                    e.preset_name.trim().into(),
                    Recipe {
                        task: e.config.task,
                        route: e.config.route.unwrap(),
                    },
                ));
            }
            if ui.button("Delete named route preset").clicked() {
                action = Some(PresetAction::Delete(e.preset_name.trim().into()));
            }
        });
    });
    if let Err(error) = drawer_core::quick_filters::inspect(
        d,
        target,
        Recipe {
            task: e.config.task,
            route: e.config.route.unwrap(),
        },
    ) {
        ui.colored_label(egui::Color32::YELLOW, error);
    }
    action
}
pub(super) fn preset(app: &mut Drawer, action: PresetAction) -> Result<(), String> {
    let old = app.workspaces.store.current.quick_presets.clone();
    let mut next = old.clone();
    match action {
        PresetAction::Save(name, recipe) => {
            if let Some((_, r)) = next.values.iter_mut().find(|(n, _)| *n == name) {
                *r = recipe;
            } else {
                next.values.push((name, recipe));
            }
        }
        PresetAction::Delete(name) => next.values.retain(|(n, _)| *n != name),
    }
    if !next.valid() {
        return Err("Use valid parameters/routes, a nonempty preset name up to64 characters, and at most32 unique names".into());
    }
    if next == old {
        return Ok(());
    }
    app.workspaces.store.current.quick_presets = next;
    if let Err(error) = app.persist_workspace() {
        app.workspaces.store.current.quick_presets = old;
        return Err(error);
    }
    Ok(())
}
