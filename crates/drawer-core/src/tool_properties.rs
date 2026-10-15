//! Per-tool property values and presentation; copying is an explicit preview transaction.
use crate::assets::BrushTip;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tool {
    Pen,
    Pencil,
    Eraser,
    Wash,
    Eyedropper,
    Move,
}
impl Tool {
    pub const ALL: [Self; 6] = [
        Self::Pen,
        Self::Pencil,
        Self::Eraser,
        Self::Wash,
        Self::Eyedropper,
        Self::Move,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Pen => "Pen",
            Self::Pencil => "Pencil",
            Self::Eraser => "Eraser",
            Self::Wash => "Wash",
            Self::Eyedropper => "Eyedropper",
            Self::Move => "Move",
        }
    }
    pub fn brush(tip: BrushTip) -> Self {
        match tip {
            BrushTip::Pen => Self::Pen,
            BrushTip::Pencil => Self::Pencil,
            BrushTip::Eraser => Self::Eraser,
            BrushTip::Wash => Self::Wash,
        }
    }
    pub fn group(self) -> u8 {
        match self {
            Self::Pen | Self::Pencil => 0,
            Self::Eraser => 1,
            Self::Wash => 2,
            Self::Eyedropper => 3,
            Self::Move => 4,
        }
    }
    pub fn bounds(self, p: Property) -> Option<[f64; 2]> {
        match self {
            Self::Eyedropper | Self::Move => None,
            _ => Some(p.bounds()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Property {
    Size,
    Aspect,
    Angle,
    Stabilization,
}
impl Property {
    pub const ALL: [Self; 4] = [Self::Size, Self::Aspect, Self::Angle, Self::Stabilization];
    pub fn name(self) -> &'static str {
        match self {
            Self::Size => "Size",
            Self::Aspect => "Aspect",
            Self::Angle => "Tip angle",
            Self::Stabilization => "Stabilization",
        }
    }
    pub fn bounds(self) -> [f64; 2] {
        match self {
            Self::Size => [0.1, 4096.0],
            Self::Aspect => [0.05, 1.0],
            Self::Angle => [0.0, 360.0],
            Self::Stabilization => [0.0, 0.95],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    Slider,
    Number,
    Indicator,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub value: f64,
    pub visible: bool,
    pub control: Control,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub tools: BTreeMap<Tool, BTreeMap<Property, State>>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            tools: Tool::ALL
                .into_iter()
                .map(|t| {
                    (
                        t,
                        Property::ALL
                            .into_iter()
                            .filter(|p| t.bounds(*p).is_some())
                            .map(|p| {
                                (
                                    p,
                                    State {
                                        value: match p {
                                            Property::Size => 12.0,
                                            Property::Aspect => 1.0,
                                            _ => 0.0,
                                        },
                                        visible: true,
                                        control: Control::Number,
                                    },
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyKind {
    Values,
    Appearance,
}
#[derive(Clone)]
pub struct Preview {
    pub next: Settings,
    pub reports: Vec<String>,
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.tools.len() == Tool::ALL.len()
            && Tool::ALL.into_iter().all(|t| {
                self.tools.get(&t).is_some_and(|props| {
                    props.len()
                        == Property::ALL
                            .into_iter()
                            .filter(|p| t.bounds(*p).is_some())
                            .count()
                        && props.iter().all(|(p, s)| {
                            t.bounds(*p).is_some_and(|[min, max]| {
                                s.value.is_finite() && s.value >= min && s.value <= max
                            })
                        })
                })
            })
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        if std::fs::metadata(path)?.len() > 65536 {
            return Err(io::Error::other("Tool property preset exceeds 64 KiB"));
        }
        let settings: Self =
            serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        if !settings.valid() {
            return Err(io::Error::other("Invalid tool property preset"));
        }
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if !self.valid() {
            return Err(io::Error::other("Invalid tool property preset"));
        }
        crate::workspaces::write_json(path, self)
    }
    pub fn preview(
        &self,
        source: Tool,
        targets: &[Tool],
        properties: &[Property],
        kind: CopyKind,
    ) -> Result<Preview, String> {
        self.preview_with_bounds(source, targets, properties, kind, Tool::bounds)
    }
    /// Target definitions can narrow their ranges without changing the copy transaction.
    pub fn preview_with_bounds(
        &self,
        source: Tool,
        targets: &[Tool],
        properties: &[Property],
        kind: CopyKind,
        bounds: impl Fn(Tool, Property) -> Option<[f64; 2]>,
    ) -> Result<Preview, String> {
        if !self.valid() || targets.is_empty() || properties.is_empty() {
            return Err("Select valid properties and at least one target".into());
        }
        let mut next = self.clone();
        let mut reports = Vec::new();
        let mut seen = BTreeSet::new();
        for target in targets.iter().copied().filter(|t| *t != source) {
            if !seen.insert(target) {
                continue;
            }
            for p in properties.iter().copied().collect::<BTreeSet<_>>() {
                let Some(s) = self.tools[&source].get(&p) else {
                    reports.push(format!(
                        "{}: {} unsupported by source",
                        source.name(),
                        p.name()
                    ));
                    continue;
                };
                let Some(t) = next.tools.get_mut(&target).unwrap().get_mut(&p) else {
                    reports.push(format!(
                        "{}: {} skipped (unsupported)",
                        target.name(),
                        p.name()
                    ));
                    continue;
                };
                let Some([min, max]) = bounds(target, p) else {
                    reports.push(format!(
                        "{}: {} skipped (unsupported)",
                        target.name(),
                        p.name()
                    ));
                    continue;
                };
                let [global_min, global_max] = p.bounds();
                if !min.is_finite()
                    || !max.is_finite()
                    || min > max
                    || min < global_min
                    || max > global_max
                {
                    return Err("Invalid target property range".into());
                }
                match kind {
                    CopyKind::Values => {
                        t.value = s.value.clamp(min, max);
                        reports.push(format!(
                            "{} · {}: {} → {}{}",
                            target.name(),
                            p.name(),
                            self.tools[&target][&p].value,
                            t.value,
                            if t.value != s.value {
                                " (limited to target range)"
                            } else {
                                ""
                            }
                        ));
                    }
                    CopyKind::Appearance => {
                        t.visible = s.visible;
                        t.control = s.control;
                        reports.push(format!(
                            "{} · {}: visible={}, {:?}",
                            target.name(),
                            p.name(),
                            t.visible,
                            t.control
                        ));
                    }
                }
            }
        }
        if reports.is_empty() {
            return Err("Choose a different target tool".into());
        }
        Ok(Preview { next, reports })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn value_and_display_copies_skip_auxiliary_tools_and_preserve_other_fields() {
        let mut s = Settings::default();
        s.tools
            .get_mut(&Tool::Pen)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap()
            .value = 300.0;
        s.tools
            .get_mut(&Tool::Pen)
            .unwrap()
            .get_mut(&Property::Size)
            .unwrap()
            .visible = false;
        let before = s.clone();
        let p = s
            .preview(Tool::Pen, &Tool::ALL, &[Property::Size], CopyKind::Values)
            .unwrap();
        assert_eq!(s, before);
        assert_eq!(p.next.tools[&Tool::Eraser][&Property::Size].value, 300.0);
        assert!(p.next.tools[&Tool::Eraser][&Property::Size].visible);
        assert_eq!(
            p.next.tools[&Tool::Eraser][&Property::Aspect],
            s.tools[&Tool::Eraser][&Property::Aspect]
        );
        assert!(p.next.tools[&Tool::Eyedropper].is_empty() && p.next.tools[&Tool::Move].is_empty());
        assert_eq!(
            p.reports
                .iter()
                .filter(|s| s.contains("unsupported"))
                .count(),
            2
        );
        let p = s
            .preview(
                Tool::Pen,
                &[Tool::Eraser],
                &[Property::Size],
                CopyKind::Appearance,
            )
            .unwrap();
        assert_eq!(p.next.tools[&Tool::Eraser][&Property::Size].value, 12.0);
        assert!(!p.next.tools[&Tool::Eraser][&Property::Size].visible);
        let p = s
            .preview_with_bounds(
                Tool::Pen,
                &[Tool::Pencil],
                &[Property::Size],
                CopyKind::Values,
                |t, p| {
                    if t == Tool::Pencil && p == Property::Size {
                        Some([1.0, 20.0])
                    } else {
                        t.bounds(p)
                    }
                },
            )
            .unwrap();
        assert_eq!(p.next.tools[&Tool::Pencil][&Property::Size].value, 20.0);
        assert!(p.reports[0].contains("limited"));
        assert!(p.next.valid());
        assert_eq!(s, before);
        assert!(
            s.preview(Tool::Pen, &[], &[Property::Size], CopyKind::Values)
                .is_err()
        );
        assert!(
            s.preview_with_bounds(
                Tool::Pen,
                &[Tool::Pencil],
                &[Property::Size],
                CopyKind::Values,
                |_, _| Some([f64::NAN, 20.0])
            )
            .is_err()
        );
        let mut bad = s.clone();
        bad.tools
            .get_mut(&Tool::Move)
            .unwrap()
            .insert(Property::Size, s.tools[&Tool::Pen][&Property::Size]);
        assert!(!bad.valid());
    }
}
