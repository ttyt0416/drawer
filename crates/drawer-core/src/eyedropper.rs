//! Raw layer sampling is independent of selection/paper; composite sampling uses visible output.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    LayerRgb,
    LayerRgba,
    #[default]
    Composite,
}
impl Source {
    pub const ALL: [Self; 3] = [Self::LayerRgb, Self::LayerRgba, Self::Composite];
    pub fn label(self) -> &'static str {
        match self {
            Self::LayerRgb => "Current layer RGB (keep drawing alpha)",
            Self::LayerRgba => "Current layer RGBA",
            Self::Composite => "Composite RGBA",
        }
    }
}
/// Local composite references. The all-raster option bypasses explicit inclusions;
/// explicit exclusions win. A scope prevents applying another document's indices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct References {
    pub enabled: bool,
    pub scope: Option<String>,
    pub all_rasters: bool,
    pub include: Vec<usize>,
    pub exclude: Vec<usize>,
    pub objects: bool,
    pub vectors: bool,
    pub animation: bool,
}
impl Default for References {
    fn default() -> Self {
        Self {
            enabled: false,
            scope: None,
            all_rasters: true,
            include: vec![],
            exclude: vec![],
            objects: true,
            vectors: true,
            animation: true,
        }
    }
}
pub fn scope(doc: &crate::document::Document) -> String {
    doc.path.as_ref().map_or_else(
        || format!("unsaved:{}:{}", std::process::id(), doc.id),
        |p| p.to_string_lossy().into_owned(),
    )
}
impl References {
    pub fn valid(&self) -> bool {
        let valid = |v: &[usize]| {
            v.len() <= 256
                && v.iter().all(|n| *n < 256)
                && v.iter().collect::<std::collections::BTreeSet<_>>().len() == v.len()
        };
        valid(&self.include)
            && valid(&self.exclude)
            && self
                .scope
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 32768 && !s.contains('\0'))
    }
    pub fn active(&self, doc: &crate::document::Document) -> bool {
        self.enabled && self.scope.as_ref().is_none_or(|s| *s == scope(doc))
    }
    pub fn raster(&self, index: usize) -> bool {
        (self.all_rasters || self.include.contains(&index)) && !self.exclude.contains(&index)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub source: Source,
    pub radius: u8,
}
impl Settings {
    pub fn valid(self) -> bool {
        self.radius <= 4
    }
    pub fn sample(
        self,
        doc: &crate::document::Document,
        layer: Option<usize>,
        point: [f64; 2],
        current: [u8; 4],
    ) -> Result<Option<[u8; 4]>, String> {
        self.sample_with(doc, layer, point, current, &References::default())
    }
    pub fn sample_with(
        self,
        doc: &crate::document::Document,
        layer: Option<usize>,
        point: [f64; 2],
        current: [u8; 4],
        references: &References,
    ) -> Result<Option<[u8; 4]>, String> {
        if !references.valid() {
            return Err("Invalid sampling references".into());
        }
        if self.source == Source::Composite
            && references.active(doc)
            && references
                .include
                .iter()
                .chain(&references.exclude)
                .any(|n| *n >= doc.layers.len())
        {
            return Err("Sample layer list changed; refresh reference layers".into());
        }
        if !self.valid()
            || point.iter().any(|v| !v.is_finite())
            || point[0] < 0.0
            || point[1] < 0.0
            || point[0] >= doc.width as f64
            || point[1] >= doc.height as f64
        {
            return Err("Sample is outside the canvas or settings are invalid".into());
        }
        let layer = if self.source != Source::Composite {
            Some(
                doc.layers
                    .get(layer.ok_or("Select a raster layer to sample")?)
                    .ok_or("Sample layer is unavailable")?,
            )
        } else {
            None
        };
        let renderer = layer
            .is_none()
            .then(|| crate::render::Renderer::sampling(doc, references));
        let center = point.map(|p| p.floor() as i64);
        let r = self.radius as i64;
        let mut count = 0u64;
        let mut sum = [0u64; 4];
        let mut weighted = [0u64; 3];
        let mut has_color = false;
        for y in (center[1] - r).max(0)..=(center[1] + r).min(doc.height as i64 - 1) {
            for x in (center[0] - r).max(0)..=(center[0] + r).min(doc.width as i64 - 1) {
                let pixel = layer.map_or_else(
                    || {
                        renderer
                            .as_ref()
                            .unwrap()
                            .pixel([x as f64 + 0.5, y as f64 + 0.5])
                    },
                    |l| l.pixel(doc.width, doc.height, x, y),
                );
                has_color |= pixel != [0; 4];
                count += 1;
                for c in 0..4 {
                    sum[c] += pixel[c] as u64;
                }
                for c in 0..3 {
                    weighted[c] += pixel[c] as u64 * pixel[3] as u64;
                }
            }
        }
        if !has_color {
            return Ok(None);
        }
        let mut result = sum.map(|v| ((v + count / 2) / count) as u8);
        // RGB-only is the arithmetic mean of stored, unassociated RGB, including hidden RGB.
        // RGBA/composite averages are alpha weighted; all-transparent stored RGB remains available.
        if self.source != Source::LayerRgb && sum[3] > 0 {
            for c in 0..3 {
                result[c] = ((weighted[c] + sum[3] / 2) / sum[3]) as u8;
            }
        }
        if self.source == Source::LayerRgb {
            result[3] = current[3];
        }
        Ok(Some(result))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default)]
    pub references: References,
    #[serde(default = "hide_screen_default")]
    pub hide_screen_window: bool,
    pub settings: Settings,
    pub recent: Vec<[u8; 4]>,
    undo: Vec<[u8; 4]>,
    redo: Vec<[u8; 4]>,
    last: Option<[u8; 4]>,
}
fn hide_screen_default() -> bool {
    true
}
impl Default for State {
    fn default() -> Self {
        Self {
            references: Default::default(),
            hide_screen_window: true,
            settings: Default::default(),
            recent: vec![],
            undo: vec![],
            redo: vec![],
            last: Some([0, 0, 0, 255]),
        }
    }
}
fn push(stack: &mut Vec<[u8; 4]>, color: [u8; 4]) {
    if stack.len() == 32 {
        stack.remove(0);
    }
    stack.push(color);
}
impl State {
    pub fn valid(&self) -> bool {
        self.settings.valid()
            && self.references.valid()
            && self.recent.len() <= 32
            && self.undo.len() <= 32
            && self.redo.len() <= 32
    }
    fn remember(&mut self, c: [u8; 4]) {
        self.recent.retain(|v| *v != c);
        self.recent.insert(0, c);
        self.recent.truncate(32);
    }
    pub fn observe(&mut self, color: [u8; 4]) {
        if let Some(old) = self.last
            && old != color
        {
            push(&mut self.undo, old);
            self.redo.clear();
            self.remember(color);
        }
        self.last = Some(color);
    }
    pub fn undo(&mut self, current: [u8; 4]) -> Option<[u8; 4]> {
        let target = self.undo.pop()?;
        push(&mut self.redo, current);
        self.last = Some(target);
        self.remember(target);
        Some(target)
    }
    pub fn redo(&mut self, current: [u8; 4]) -> Option<[u8; 4]> {
        let target = self.redo.pop()?;
        push(&mut self.undo, current);
        self.last = Some(target);
        self.remember(target);
        Some(target)
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> crate::document::Document {
        let mut d = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: 3,
                height: 1,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            2048,
        )
        .unwrap();
        d.layers[0].pixels = vec![[20, 80, 160, 0], [20, 80, 160, 128], [20, 80, 160, 255]];
        d
    }
    #[test]
    fn stored_rgb_rgba_zero_half_full_and_composite_are_distinct_without_document_changes() {
        let mut d = doc();
        d.scene.selection_mask = Some(Default::default());
        d.scene.paper.layers.insert(
            0,
            crate::paper::Paper {
                pattern: crate::texture::Pattern {
                    source: 1,
                    strength: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let before = d.snapshot();
        for (x, alpha) in [(0, 0), (1, 128), (2, 255)] {
            assert_eq!(
                Settings {
                    source: Source::LayerRgb,
                    radius: 0
                }
                .sample(&d, Some(0), [x as f64 + 0.5, 0.5], [1, 2, 3, 47])
                .unwrap(),
                Some([20, 80, 160, 47])
            );
            assert_eq!(
                Settings {
                    source: Source::LayerRgba,
                    radius: 0
                }
                .sample(&d, Some(0), [x as f64 + 0.5, 0.5], [1, 2, 3, 47])
                .unwrap(),
                Some([20, 80, 160, alpha])
            );
            assert_eq!(
                Settings::default()
                    .sample(&d, None, [x as f64 + 0.5, 0.5], [1, 2, 3, 47])
                    .unwrap(),
                if alpha == 0 {
                    None
                } else {
                    Some(d.composite_pixel(x, 0))
                }
            );
        }
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
    }
    #[test]
    fn single_average_edges_hidden_rgb_and_invalid_inputs() {
        let mut d = doc();
        d.layers[0].pixels = vec![[255, 0, 0, 0], [0, 0, 255, 255], [0; 4]];
        assert_eq!(
            Settings {
                source: Source::LayerRgb,
                radius: 1
            }
            .sample(&d, Some(0), [0.5, 0.5], [0, 0, 0, 53])
            .unwrap(),
            Some([128, 0, 128, 53])
        );
        assert_eq!(
            Settings {
                source: Source::LayerRgba,
                radius: 1
            }
            .sample(&d, Some(0), [0.5, 0.5], [0; 4])
            .unwrap(),
            Some([0, 0, 255, 128])
        );
        assert_eq!(
            Settings {
                source: Source::LayerRgba,
                radius: 0
            }
            .sample(&d, Some(0), [0.5, 0.5], [0; 4])
            .unwrap(),
            Some([255, 0, 0, 0])
        );
        assert_eq!(
            Settings {
                source: Source::LayerRgba,
                radius: 0
            }
            .sample(&d, Some(0), [2.5, 0.5], [0; 4])
            .unwrap(),
            None
        );
        for point in [[-1.0, 0.0], [3.0, 0.0], [f64::NAN, 0.0]] {
            assert!(Settings::default().sample(&d, None, point, [0; 4]).is_err());
        }
        assert!(
            Settings {
                source: Source::LayerRgb,
                radius: 0
            }
            .sample(&d, None, [0.5, 0.5], [0; 4])
            .is_err()
        );
        assert!(
            !Settings {
                radius: 5,
                ..Default::default()
            }
            .valid()
        );
    }
    #[test]
    fn hidden_current_layer_samples_raw_data_while_composite_excludes_it() {
        let mut d = doc();
        d.scene.paper.layers.insert(
            0,
            crate::paper::Paper {
                hidden: true,
                blend: crate::brush_composite::Mode::ReplaceAlpha,
                ..Default::default()
            },
        );
        let before = d.snapshot();
        assert_eq!(
            Settings {
                source: Source::LayerRgba,
                radius: 0
            }
            .sample(&d, Some(0), [1.5, 0.5], [0; 4])
            .unwrap(),
            Some([20, 80, 160, 128])
        );
        assert_eq!(
            Settings::default()
                .sample(&d, None, [1.5, 0.5], [0; 4])
                .unwrap(),
            None
        );
        assert_eq!(d.layers[0].pixels[1], [20, 80, 160, 128]);
        assert!(crate::paper::bake(&mut d, crate::paper::Node::Raster(0)).is_err());
        assert!(d.snapshot() == before);
        assert!(!crate::psd::layers_supported(&d));
        let root = crate::test_support::TempDir::new();
        let path = root.0.join("hidden.drawer");
        crate::files::save(&mut d, &path).unwrap();
        let reopened = crate::files::open(&path, 8192).unwrap();
        assert!(reopened.snapshot() == before);
        assert_eq!(
            Settings {
                source: Source::LayerRgba,
                radius: 0
            }
            .sample(&reopened, Some(0), [1.5, 0.5], [0; 4])
            .unwrap(),
            Some([20, 80, 160, 128])
        );
        assert_eq!(reopened.composite_pixel(1, 0), [0; 4]);
    }
    #[test]
    fn color_history_is_bounded_persistent_and_separate_from_drawing() {
        let mut s = State::default();
        s.observe([0; 4]);
        for n in 1..100 {
            s.observe([n; 4]);
        }
        assert!(s.valid());
        assert_eq!(s.recent.len(), 32);
        assert_eq!(s.undo([99; 4]), Some([98; 4]));
        assert_eq!(s.redo([98; 4]), Some([99; 4]));
        let mut copy: State = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(copy, s);
        assert_eq!(copy.undo([99; 4]), Some([98; 4]));
        copy.observe([50; 4]);
        assert!(!copy.can_redo());
        copy.recent.resize(33, [0; 4]);
        assert!(!copy.valid());
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    fn references_exclusions_alpha_weighting_and_source_isolation() {
        let mut d = crate::document::Document::create(
            crate::document::CanvasSpec {
                width: 3,
                height: 3,
                dpi: 300.0,
                background: crate::document::Background::Transparent,
            },
            4096,
        )
        .unwrap();
        d.layers[0].pixels = vec![[0, 0, 255, 0]; 9];
        d.layers[0].pixels[0] = [255, 0, 0, 128];
        d.layers[0].pixels[1] = [0, 255, 0, 255];
        d.layers[0].pixels[2] = [0, 255, 0, 255];
        d.layers.push(crate::document::RasterLayer {
            locked: true,
            name: "Overlay".into(),
            pixels: vec![[0, 0, 255, 255]; 9],
            placement: None,
        });
        let before = d.snapshot();
        let settings = Settings {
            source: Source::Composite,
            radius: 1,
        };
        let mut r = References {
            enabled: true,
            all_rasters: false,
            include: vec![0],
            ..Default::default()
        };
        assert_eq!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .unwrap(),
            Some([51, 204, 0, 71])
        );
        r.include.push(1);
        assert_eq!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .unwrap(),
            Some([0, 0, 255, 255])
        );
        r.exclude.push(1);
        assert_eq!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .unwrap(),
            Some([51, 204, 0, 71])
        );
        r.exclude.push(0);
        assert_eq!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .unwrap(),
            None
        );
        assert_eq!(
            Settings {
                source: Source::LayerRgb,
                radius: 1
            }
            .sample_with(&d, Some(0), [1.5, 1.5], [0, 0, 0, 47], &r)
            .unwrap(),
            Some([28, 57, 170, 47])
        );
        r.exclude.clear();
        r.scope = Some("another document".into());
        assert_eq!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .unwrap(),
            Some([0, 0, 255, 255])
        );
        r.scope = None;
        r.include = vec![];
        assert_eq!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .unwrap(),
            None
        );
        r.include.push(255);
        assert!(
            settings
                .sample_with(&d, None, [1.5, 1.5], [0; 4], &r)
                .is_err()
        );
        assert!(
            Settings {
                source: Source::LayerRgb,
                radius: 0
            }
            .sample_with(&d, Some(0), [0.5, 0.5], [0; 4], &r)
            .is_ok()
        );
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
    }
    #[test]
    fn vector_object_switches_and_legacy_state_defaults() {
        let mut d = crate::line_tools_verify::fixture().unwrap();
        let mut r = References {
            enabled: true,
            all_rasters: false,
            ..Default::default()
        };
        let s = Settings::default();
        assert_eq!(
            s.sample_with(&d, None, [8.0, 20.0], [0; 4], &r).unwrap(),
            Some([0, 0, 0, 255])
        );
        r.vectors = false;
        assert_eq!(
            s.sample_with(&d, None, [8.0, 20.0], [0; 4], &r).unwrap(),
            None
        );
        d.scene.objects.push(crate::scene::PageObject::Line {
            start: [4.0, 20.5],
            end: [60.0, 20.5],
            width_px: 2.0,
            color: [20, 80, 160, 128],
        });
        assert_eq!(
            s.sample_with(&d, None, [8.0, 20.0], [0; 4], &r).unwrap(),
            Some([20, 80, 160, 128])
        );
        r.objects = false;
        assert_eq!(
            s.sample_with(&d, None, [8.0, 20.0], [0; 4], &r).unwrap(),
            None
        );
        let mut state = serde_json::to_value(State::default()).unwrap();
        state.as_object_mut().unwrap().remove("references");
        state.as_object_mut().unwrap().remove("hide_screen_window");
        let state: State = serde_json::from_value(state).unwrap();
        assert!(!state.references.enabled);
        assert!(state.hide_screen_window && state.valid());
    }
}
