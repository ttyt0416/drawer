//! Explicit input/output routes. Presets contain values and kinds, never document IDs.
use crate::{
    batch_filters::Task,
    document::{
        Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document, PlacedRaster, RasterLayer,
    },
    layer_properties::{self as props, Target},
    sizing::DocumentState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Quick,
    Pixels,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    #[default]
    Current,
    Composite,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Output {
    Current,
    #[default]
    NewLayer,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Raster,
    Vector,
    Object,
    Gradient,
    World,
    Folder,
    Composite,
}
pub fn kind(t: Target) -> Option<Kind> {
    Some(match t {
        Target::Raster(_) => Kind::Raster,
        Target::Vector(_) => Kind::Vector,
        Target::Object(_) => Kind::Object,
        Target::Gradient(_) => Kind::Gradient,
        Target::World(_) => Kind::World,
        Target::Folder(_) => Kind::Folder,
        _ => return None,
    })
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Route {
    pub mode: Mode,
    pub input: Input,
    pub output: Output,
    pub kind: Kind,
}
impl Route {
    pub fn valid(self) -> bool {
        (self.mode != Mode::Quick || self.output == Output::NewLayer)
            && (if self.input == Input::Composite {
                self.output == Output::NewLayer && self.kind == Kind::Composite
            } else {
                self.kind != Kind::Composite
            })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub task: Task,
    pub route: Route,
}
impl Recipe {
    pub fn valid(self) -> bool {
        self.task.valid() && self.route.valid()
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Presets {
    pub values: Vec<(String, Recipe)>,
}
impl Presets {
    pub fn is_default(&self) -> bool {
        self.values.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.values.len() <= 32
            && self.values.iter().enumerate().all(|(i, (n, r))| {
                !n.trim().is_empty()
                    && n.chars().count() <= 64
                    && !n.chars().any(char::is_control)
                    && r.valid()
                    && !self.values[..i].iter().any(|(old, _)| old == n)
            })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub recipe: Recipe,
    pub source: Target,
    pub result: Target,
    pub algorithm: u16,
}
impl Record {
    pub fn valid(&self) -> bool {
        let source_valid = match self.source {
            Target::Raster(id)
            | Target::Vector(id)
            | Target::Object(id)
            | Target::Gradient(id)
            | Target::World(id)
            | Target::Selection(id) => id > 0,
            Target::Folder(id) => id < 256,
            Target::Track(id) => id < 32,
            Target::Cell { track, cell } => track < 32 && cell < 10000,
        };
        self.recipe.valid()
            && source_valid
            && matches!(self.result,Target::Raster(id)if id>0)
            && (self.recipe.route.input == Input::Composite
                || kind(self.source) == Some(self.recipe.route.kind))
            && self.algorithm == 1
            && (self.recipe.route.output != Output::NewLayer || self.source != self.result)
    }
}
pub struct Prepared {
    pub state: DocumentState,
    pub selected: Target,
    pub notes: Vec<String>,
    pub loss: bool,
}
pub fn inspect(d: &Document, target: Target, recipe: Recipe) -> Result<(), String> {
    if !recipe.valid() {
        return Err("Invalid filter task or input/output route".into());
    }
    if recipe.route.input == Input::Current {
        if !props::targets(d).contains(&target) {
            return Err("Source target is missing".into());
        }
        if kind(target) != Some(recipe.route.kind) {
            return Err(format!(
                "Preset target type {:?} differs from selected {:?}; adapt the type or select a compatible layer",
                recipe.route.kind,
                kind(target)
            ));
        }
        if recipe.route.output == Output::Current
            && let Target::Raster(id) = target
        {
            props::paintable(d, crate::layers::index(d, id).ok_or("Missing raster")?)?;
        }
    }
    Ok(())
}
fn budget(d: &Document, available: usize, count: usize) -> Result<(), String> {
    if count > 4 * 1024 * 1024
        || d.memory_bytes() > DEFAULT_DOCUMENT_BUDGET / 8
        || d.resident_bytes()
            .saturating_add(d.memory_bytes().saturating_mul(5))
            .saturating_add(count.saturating_mul(64))
            .saturating_add(32 * 1024 * 1024)
            > available.min(DEFAULT_DOCUMENT_BUDGET)
    {
        return Err("Filter route source4M/snapshot32MiB/workspace memory limit".into());
    }
    Ok(())
}
pub fn composite(
    d: &Document,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<RasterLayer, String> {
    let count = d.width as usize * d.height as usize;
    budget(d, DEFAULT_DOCUMENT_BUDGET, count)?;
    let complexity = d.layers.len()
        + d.scene.objects.len() * 4
        + d.scene
            .lines
            .layers
            .iter()
            .flat_map(|l| &l.paths)
            .map(|p| p.points.len())
            .sum::<usize>()
        + d.scene.worlds.iter().map(|w| w.edges.len()).sum::<usize>();
    if count.saturating_mul(complexity.max(1)) > 64 * 1024 * 1024 {
        return Err("Composite input exceeds64M work limit".into());
    }
    tick(0, count)?;
    let legacy = (d.color.calculation == crate::color_storage::Calculation::LegacyEight)
        .then(|| crate::render::Renderer::new(d));
    let float = if legacy.is_none() {
        Some(crate::float_render::Renderer::with_effect_progress(
            d, None, &mut tick,
        )?)
    } else {
        None
    };
    let mut samples = Vec::with_capacity(count);
    for y in 0..d.height {
        for x in 0..d.width {
            let i = y as usize * d.width as usize + x as usize;
            if i.is_multiple_of(128) {
                tick(i, count)?;
            }
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            samples.push(if let Some(r) = &legacy {
                r.sample(p)?
            } else {
                float.as_ref().unwrap().pixel(p)?
            });
        }
    }
    let e = PlacedRaster {
        offset: [0; 2],
        width: d.width,
        height: d.height,
    };
    let mut raster = d.layers[0].clone();
    raster.name = "Composite input".into();
    raster.locked = false;
    raster.placement = None;
    crate::color_storage::set_samples(&mut raster, &d.color, e, samples)?;
    Ok(raster)
}
fn scratch(d: &Document, raster: RasterLayer) -> Result<Document, String> {
    let mut temp = Document::create(
        CanvasSpec {
            width: d.width,
            height: d.height,
            dpi: d.dpi,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    temp.color = d.color.clone();
    temp.layers[0] = raster;
    temp.layers[0].locked = false;
    temp.scene.transform = d.scene.transform;
    temp.scene.selection = d.scene.selection;
    temp.scene.selection_mask = d.scene.selection_mask.clone();
    temp.scene.selection_translation = d.scene.selection_translation;
    Ok(temp)
}
fn task(
    d: &Document,
    layer: usize,
    task: Task,
    available: usize,
    tick: &mut impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<DocumentState, String> {
    match task {
        Task::Detail(s) => crate::detail_filter::prepare_with_budget(d, layer, s, available, tick),
        Task::Blur(s) => crate::spatial_blur::prepare(d, layer, s, tick),
        Task::Gaussian { radius, selection } => {
            crate::gaussian::prepare_with_progress(d, layer, radius, selection, tick)
        }
        Task::Surface {
            operation,
            selection,
        } => {
            let stack = crate::surface_effects::Stack {
                effects: vec![crate::surface_effects::Entry {
                    operation,
                    enabled: true,
                    scope: None,
                }],
                output_mask: if selection {
                    Some(crate::surface_effects::capture_output_mask(d)?)
                } else {
                    None
                },
                ..Default::default()
            };
            crate::surface_effects::rasterize(d, layer, stack, tick)
        }
    }
}
pub fn append(
    d: &Document,
    raster: RasterLayer,
    name: &str,
) -> Result<(DocumentState, Target), String> {
    let mut state = crate::layers::prepare(
        d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: name.into(),
            parent: None,
        },
    )?
    .state;
    let n = state.layers.len() - 1;
    let id = state.scene.layer_ids.id(n);
    state.layers[n] = raster;
    state.layers[n].name = name.into();
    state.layers[n].locked = false;
    state.scene.render_order.entries = crate::render_order::normalized(&d.scene);
    state
        .scene
        .render_order
        .entries
        .push(crate::render_order::Entry {
            key: crate::layer_search::Key::Layer(Target::Raster(id)),
            group: crate::render_order::Group::World,
        });
    crate::layer_groups::finish(d, &state)?;
    Ok((state, Target::Raster(id)))
}
pub fn prepare(
    d: &Document,
    target: Target,
    recipe: Recipe,
    available: usize,
    mut tick: impl FnMut(usize, usize) -> Result<(), String>,
) -> Result<Prepared, String> {
    tick(0, 1)?;
    inspect(d, target, recipe)?;
    budget(d, available, d.layers.iter().map(|l| l.pixels.len()).sum())?;
    let mut notes = vec![format!(
        "Input {:?}, target type {:?}, output {:?}.",
        recipe.route.input, recipe.route.kind, recipe.route.output
    )];
    let loss = recipe.route.input == Input::Current
        && recipe.route.output == Output::Current
        && !matches!(target, Target::Raster(_));
    if recipe.route.output == Output::Current
        && let Target::Raster(id) = target
        && !matches!(recipe.task, Task::Surface { .. })
    {
        let n = crate::layers::index(d, id).ok_or("Missing raster")?;
        let mut state = task(d, n, recipe.task, available, &mut tick)?;
        if state != d.snapshot() {
            state.scene.last_quick_filter = Some(Box::new(Record {
                recipe,
                source: target,
                result: target,
                algorithm: 1,
            }));
            crate::layer_groups::finish(d, &state)?;
        }
        return Ok(Prepared {
            state,
            selected: target,
            notes,
            loss: false,
        });
    }
    if recipe.route.input == Input::Current && matches!(target, Target::Raster(_)) {
        notes.push("Current raster input uses stored pixels before live effects. Entire composite input includes the displayed effects and masks.".into());
    }
    let mut converted = None;
    let input = if recipe.route.input == Input::Composite {
        composite(d, &mut tick)?
    } else if let Target::Raster(id) = target {
        d.layers[crate::layers::index(d, id).ok_or("Missing raster")?].clone()
    } else {
        let key = crate::layer_catalog::key(d, target);
        let p = crate::raster_conversion::prepare(
            d,
            key,
            crate::raster_conversion::Action::Rasterize(crate::layer_search::Expression::Color),
        )?;
        if loss {
            notes.extend(p.notes);
            notes.push(format!(
                "Conversion includes {} linked source entries.",
                p.affected.len()
            ));
        } else {
            notes.push("The selected editable source is rendered for the new layer; its original edit information remains intact.".into());
        }
        let crate::layer_search::Key::Layer(Target::Raster(id)) = p.selected else {
            return Err("Raster conversion result mismatch".into());
        };
        let n = (0..p.state.layers.len())
            .find(|n| p.state.scene.layer_ids.id(*n) == id)
            .ok_or("Missing converted input")?;
        let raster = p.state.layers[n].clone();
        converted = Some((p.state, n, id));
        raster
    };
    budget(d, available, input.pixels.len())?;
    let working = scratch(d, input)?;
    let filtered = task(&working, 0, recipe.task, available, &mut tick)?;
    let raster = filtered
        .layers
        .last()
        .ok_or("Missing filter output")?
        .clone();
    let (mut state, selected) = if recipe.route.output == Output::NewLayer {
        notes.push("Sources remain visible and editable. The new Normal/100% result sits above all sources; transparent areas can become denser. Review the page preview.".into());
        append(d, raster, "Quick filter result")?
    } else if let Some((mut state, n, id)) = converted {
        state.layers[n] = raster;
        state.layers[n].name = "Filtered raster conversion".into();
        notes.push("This replaces the selected editable source (and any converted clipping run) with raster pixels. Its text/vector/material/geometry and live edit information is lost. Confirm this loss only after reviewing the preview. One Undo restores every source.".into());
        (state, Target::Raster(id))
    } else {
        let Target::Raster(id) = target else {
            return Err("Current output needs a raster conversion".into());
        };
        let n = crate::layers::index(d, id).ok_or("Missing raster")?;
        if crate::clipping::alpha_locked(d, n) {
            return Err(
                "Unlock transparency before baking a surface effect into current pixels".into(),
            );
        }
        props::paintable(d, n)?;
        let mut state = d.snapshot();
        let name = state.layers[n].name.clone();
        state.layers[n] = raster;
        state.layers[n].name = name;
        (state, target)
    };
    if let Some(s) = filtered.scene.last_detail_filter {
        state.scene.last_detail_filter = Some(s);
    }
    if let Some(s) = filtered.scene.last_spatial_blur {
        state.scene.last_spatial_blur = Some(s);
    }
    state.scene.last_quick_filter = Some(Box::new(Record {
        recipe,
        source: target,
        result: selected,
        algorithm: 1,
    }));
    crate::layer_groups::finish(d, &state)?;
    tick(1, 1)?;
    Ok(Prepared {
        state,
        selected,
        notes,
        loss,
    })
}
pub fn factory(kind: Kind) -> Vec<(String, Recipe)> {
    let route = Route {
        kind,
        ..Default::default()
    };
    vec![
        (
            "Line contrast".into(),
            Recipe {
                task: Task::default(),
                route,
            },
        ),
        (
            "Soft blur".into(),
            Recipe {
                task: Task::Gaussian {
                    radius: 3,
                    selection: false,
                },
                route,
            },
        ),
        (
            "Warm glow".into(),
            Recipe {
                task: Task::Surface {
                    operation: crate::surface_effects::Operation::defaults()[1],
                    selection: false,
                },
                route,
            },
        ),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectAlgorithm(pub u16);
impl Default for EffectAlgorithm {
    fn default() -> Self {
        Self(1)
    }
}
pub fn attached_effects(d: &Document, target: crate::layers::Target) -> bool {
    let t = match target {
        crate::layers::Target::Raster(id) => Target::Raster(id),
        crate::layers::Target::Vector(id) => Target::Vector(id),
    };
    matches!(t,Target::Raster(id)if d.scene.surface_effects.rasters.get(&id).is_some_and(crate::surface_effects::Stack::active))
        || d.scene.layer_effects.get(t).is_some_and(|e| {
            e.adjustment.is_some_and(|a| !a.identity())
                || e.mask.as_ref().is_some_and(|m| m.enabled)
        })
}
pub fn delete_source(
    d: &Document,
    target: crate::layers::Target,
    keep: bool,
) -> Result<crate::layers::Prepared, String> {
    if !keep {
        return crate::layers::prepare(d, crate::layers::Command::Delete(target));
    }
    let image = composite(d, |_, _| Ok(()))?;
    let (state, _) = append(d, image, "Preserved appearance before source deletion")?;
    let working = crate::layer_merge::preview(&state, d.current_frame)?;
    crate::layers::prepare(&working, crate::layers::Command::Delete(target))
}
