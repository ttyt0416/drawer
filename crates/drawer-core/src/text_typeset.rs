//! Opt-in typography and local text-tool preferences. Old layouts never gain these implicitly.
use crate::text_layout::{Settings, Style};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kerning {
    #[default]
    Manual,
    Metrics,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feature {
    Ligatures,
    ContextualAlternates,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastLine {
    #[default]
    Left,
    Center,
    Right,
    Top,
    Bottom,
}
impl LastLine {
    pub fn factor(self) -> f64 {
        match self {
            Self::Left | Self::Top => 0.,
            Self::Center => 0.5,
            Self::Right | Self::Bottom => 1.,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub kerning: Kerning,
    pub manual: f64,
    pub features: Vec<Feature>,
    pub justify: bool,
    pub last: LastLine,
    pub half_punctuation: bool,
    pub horizontal: Vec<[usize; 2]>,
    pub background: [u8; 4],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            kerning: Kerning::Manual,
            manual: 0.,
            features: vec![],
            justify: false,
            last: LastLine::Left,
            half_punctuation: false,
            horizontal: vec![],
            background: [0; 4],
        }
    }
}
impl Options {
    pub fn valid(&self, text: &str) -> bool {
        (-0.5..=4.).contains(&self.manual)
            && self.features.len() <= 2
            && self
                .features
                .iter()
                .enumerate()
                .all(|(i, v)| !self.features[..i].contains(v))
            && self.horizontal.len() <= 64
            && self.horizontal.iter().enumerate().all(|(i, r)| {
                r[0] < r[1]
                    && r[1] <= text.chars().count()
                    && r[1] - r[0] <= 16
                    && crate::text_format::boundary(text, r[0])
                    && crate::text_format::boundary(text, r[1])
                    && (i == 0 || self.horizontal[i - 1][1] <= r[0])
                    && !text.chars().skip(r[0]).take(r[1] - r[0]).any(|c| c == '\n')
            })
    }
    pub fn feature(&self, f: Feature) -> bool {
        self.features.contains(&f)
    }
    pub fn remap(&mut self, before: &str, after: &str) {
        let mut ranges: Vec<_> = self
            .horizontal
            .iter()
            .map(|r| crate::text_layout::Ruby {
                start: r[0],
                end: r[1],
                text: "x".into(),
                ..Default::default()
            })
            .collect();
        crate::text_layout::remap_ruby(before, after, &mut ranges);
        self.horizontal = ranges
            .into_iter()
            .filter_map(|r| {
                let (a, b) = crate::text_format::expand(after, r.start, r.end);
                (a < b && b - a <= 16).then_some([a, b])
            })
            .collect();
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreateAction {
    CreateLayerAlways,
    AddToSelectedText,
    #[default]
    DetectPosition,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DragAction {
    #[default]
    CreateTextBox,
    SelectText,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyTarget {
    None,
    #[default]
    NewTextOnly,
    SelectedText,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Fast,
    High,
    #[default]
    Auto,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tool {
    pub create: CreateAction,
    pub drag: DragAction,
    pub target: ApplyTarget,
    pub quality: Quality,
    pub defaults: Settings,
}
impl Default for Tool {
    fn default() -> Self {
        Self {
            create: Default::default(),
            drag: Default::default(),
            target: Default::default(),
            quality: Default::default(),
            defaults: Settings {
                baseline: [0.; 2],
                text: String::new(),
                size: 24.,
                color: [0, 0, 0, 255],
                style: Style::default(),
            },
        }
    }
}
impl Tool {
    pub fn valid(&self) -> bool {
        self.defaults.text.is_empty()
            && self.defaults.size >= 0.1
            && self.defaults.size <= 4096.
            && self.defaults.style.text_valid("")
            && self.defaults.style.frame.is_identity()
            && self.defaults.baseline == [0.; 2]
    }
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
/// Deliberately small shared policy; None and NewTextOnly never overwrite selected text.
pub fn apply_defaults(tool: &Tool, selected: bool, current: &Settings) -> Settings {
    if !matches!(
        (tool.target, selected),
        (ApplyTarget::NewTextOnly, false) | (ApplyTarget::SelectedText, true)
    ) {
        return current.clone();
    }
    let mut out = tool.defaults.clone();
    out.text = current.text.clone();
    out.baseline = current.baseline;
    if selected {
        out.style.frame = current.style.frame;
        out.style.ruby = current.style.ruby.clone();
        if current.style.rich.is_some() {
            out.style.rich = current.style.rich.clone();
        }
        if out.style.typeset.is_some() {
            out.style.generation = 2;
            out.style.rich.get_or_insert_with(Default::default);
        }
    }
    if !out.style.text_valid(&out.text) {
        out.style.remap_text("", &out.text);
    }
    out
}

pub fn punctuation(c: char) -> bool {
    matches!(
        c,
        '、' | '。'
            | '「'
            | '」'
            | '『'
            | '』'
            | '（'
            | '）'
            | '［'
            | '］'
            | '｛'
            | '｝'
            | '！'
            | '？'
            | '：'
            | '；'
    )
}
pub fn manual(style: &Style) -> f64 {
    style.typeset.as_ref().map_or(0., |o| {
        if o.kerning == Kerning::Manual {
            o.manual
        } else {
            0.
        }
    })
}
