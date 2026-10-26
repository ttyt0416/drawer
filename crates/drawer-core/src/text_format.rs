//! Grapheme-safe formatting and value snapshots of local mixed-font presets.
use crate::text_layout::Font;
use serde::{Deserialize, Serialize};
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Border {
    pub width: f64,
    pub color: [u8; 4],
}
impl Border {
    pub fn valid(self) -> bool {
        (0. ..=128.).contains(&self.width)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Attr {
    pub font: Option<Font>,
    pub ratio: Option<f64>,
    pub color: Option<[u8; 4]>,
    pub border: Option<Border>,
}
impl Attr {
    pub fn valid(self) -> bool {
        self.font.is_none_or(Font::valid)
            && self.ratio.is_none_or(|v| (0.1..=4.).contains(&v))
            && self.border.is_none_or(Border::valid)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub attr: Attr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Group {
    Korean,
    Latin,
    Digits,
    Han,
    Kana,
    Punctuation,
    Custom,
}
impl Group {
    pub const ALL: [Self; 7] = [
        Self::Korean,
        Self::Latin,
        Self::Digits,
        Self::Han,
        Self::Kana,
        Self::Punctuation,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Korean => "Hangul",
            Self::Latin => "Latin",
            Self::Digits => "Decimal digits",
            Self::Han => "Han",
            Self::Kana => "Kana",
            Self::Punctuation => "Punctuation",
            Self::Custom => "Custom Unicode range",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub group: Group,
    pub from: u32,
    pub through: u32,
    pub font: Font,
    pub ratio: f64,
}
impl Rule {
    pub fn valid(&self) -> bool {
        self.font.valid()
            && (0.1..=4.).contains(&self.ratio)
            && (self.group != Group::Custom
                || (self.from <= self.through && self.through <= 0x10ffff))
    }
    fn matches(&self, c: char) -> bool {
        match self.group {
            Group::Digits => c.general_category() == GeneralCategory::DecimalNumber,
            Group::Punctuation => c.general_category_group() == GeneralCategoryGroup::Punctuation,
            Group::Custom => (self.from..=self.through).contains(&(c as u32)),
            Group::Korean => c.script() == Script::Hangul,
            Group::Latin => c.script() == Script::Latin,
            Group::Han => c.script() == Script::Han,
            Group::Kana => matches!(c.script(), Script::Hiragana | Script::Katakana),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub base: Font,
    pub rules: Vec<Rule>,
}
impl Default for Mapping {
    fn default() -> Self {
        Self {
            base: Font::Korean,
            rules: Group::ALL
                .into_iter()
                .filter(|g| *g != Group::Custom)
                .map(|group| Rule {
                    font: if matches!(group, Group::Latin | Group::Digits) {
                        Font::Ubuntu
                    } else {
                        Font::Korean
                    },
                    group,
                    from: 0,
                    through: 0,
                    ratio: 1.,
                })
                .collect(),
        }
    }
}
impl Mapping {
    pub fn valid(&self) -> bool {
        self.base.valid() && self.rules.len() <= 16 && self.rules.iter().all(Rule::valid)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rich {
    pub spans: Vec<Span>,
    pub mapping: Option<Mapping>,
}
pub fn boundaries(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    let mut n = 0;
    for g in text.graphemes(true) {
        n += g.chars().count();
        out.push(n);
    }
    out
}
pub fn expand(text: &str, start: usize, end: usize) -> (usize, usize) {
    let b = boundaries(text);
    let end = end.min(*b.last().unwrap());
    let start = start.min(end);
    let a = b
        .iter()
        .copied()
        .take_while(|v| *v <= start)
        .last()
        .unwrap_or(0);
    let z = b.iter().copied().find(|v| *v >= end).unwrap_or(end);
    (a, z)
}
pub fn boundary(text: &str, n: usize) -> bool {
    boundaries(text).contains(&n)
}
pub fn slice(text: &str, start: usize, end: usize) -> &str {
    let a = text
        .char_indices()
        .nth(start)
        .map_or(text.len(), |(i, _)| i);
    let b = text.char_indices().nth(end).map_or(text.len(), |(i, _)| i);
    &text[a..b]
}
impl Rich {
    pub fn valid(&self, text: &str) -> bool {
        if self.spans.len() > 64 || self.mapping.as_ref().is_some_and(|m| !m.valid()) {
            return false;
        }
        let b = boundaries(text);
        let mut last = 0;
        self.spans.iter().all(|s| {
            let ok = s.start >= last
                && s.start < s.end
                && b.contains(&s.start)
                && b.contains(&s.end)
                && s.attr.valid();
            last = s.end;
            ok
        })
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of_val(self)
            + self.spans.len() * std::mem::size_of::<Span>()
            + self.mapping.as_ref().map_or(0, |m| {
                std::mem::size_of_val(m) + m.rules.len() * std::mem::size_of::<Rule>()
            })
    }
    pub fn resolve(&self, grapheme: &str, scalar: usize, base: Font) -> Attr {
        let c = grapheme
            .chars()
            .find(|c| !matches!(c.general_category_group(), GeneralCategoryGroup::Mark))
            .unwrap_or(' ');
        let mut attr = Attr {
            font: Some(self.mapping.as_ref().map_or(base, |m| m.base)),
            ratio: Some(1.),
            ..Default::default()
        };
        if let Some(rule) = self
            .mapping
            .as_ref()
            .and_then(|m| m.rules.iter().find(|r| r.matches(c)))
        {
            attr.font = Some(rule.font);
            attr.ratio = Some(rule.ratio);
        }
        if let Some(s) = self
            .spans
            .iter()
            .find(|s| s.start <= scalar && scalar < s.end)
        {
            if s.attr.font.is_some() {
                attr.font = s.attr.font;
            }
            if s.attr.ratio.is_some() {
                attr.ratio = s.attr.ratio;
            }
            attr.color = s.attr.color;
            attr.border = s.attr.border;
        }
        attr
    }
    pub fn set(
        &mut self,
        text: &str,
        start: usize,
        end: usize,
        attr: Option<Attr>,
    ) -> Result<(), String> {
        if start >= end || end > text.chars().count() || attr.is_some_and(|a| !a.valid()) {
            return Err("Choose a nonempty valid grapheme range".into());
        }
        let (start, end) = expand(text, start, end);
        let mut next = vec![];
        for s in &self.spans {
            if s.end <= start || s.start >= end {
                next.push(s.clone());
            } else {
                if s.start < start {
                    next.push(Span {
                        start: s.start,
                        end: start,
                        attr: s.attr,
                    });
                }
                if s.end > end {
                    next.push(Span {
                        start: end,
                        end: s.end,
                        attr: s.attr,
                    });
                }
            }
        }
        if let Some(attr) = attr {
            next.push(Span { start, end, attr });
        }
        next.sort_by_key(|s| s.start);
        let mut merged: Vec<Span> = vec![];
        for s in next {
            if let Some(last) = merged.last_mut()
                && last.end == s.start
                && last.attr == s.attr
            {
                last.end = s.end;
            } else {
                merged.push(s);
            }
        }
        if merged.len() > 64 {
            return Err("At most64 separate formatting ranges".into());
        }
        self.spans = merged;
        Ok(())
    }
    pub fn remap(&mut self, before: &str, after: &str) {
        let a: Vec<_> = before.chars().collect();
        let b: Vec<_> = after.chars().collect();
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let suffix = a[prefix..]
            .iter()
            .rev()
            .zip(b[prefix..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let (old, new) = (a.len() - suffix, b.len() - suffix);
        let mut last = 0;
        self.spans.retain_mut(|s| {
            if s.end > prefix {
                if s.start >= old {
                    s.start = s.start - old + new;
                    s.end = s.end - old + new;
                } else {
                    s.start = s.start.min(prefix);
                    s.end = if s.end >= old { s.end - old + new } else { new };
                }
            }
            if s.start >= s.end || s.end > b.len() {
                return false;
            }
            let (start, end) = expand(after, s.start, s.end);
            s.start = start.max(last);
            s.end = end;
            last = end;
            s.start < s.end
        });
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Presets {
    pub values: Vec<(String, Mapping)>,
}
impl Presets {
    pub fn is_default(&self) -> bool {
        self.values.is_empty()
    }
    pub fn valid(&self) -> bool {
        self.values.len() <= 32
            && self.values.iter().enumerate().all(|(i, (n, m))| {
                !n.trim().is_empty()
                    && n.chars().count() <= 64
                    && !n.chars().any(char::is_control)
                    && m.valid()
                    && !self.values[..i]
                        .iter()
                        .any(|(other, _)| other.trim().to_lowercase() == n.trim().to_lowercase())
            })
    }
    pub fn save(&mut self, name: String, mapping: Mapping, replace: bool) -> Result<(), String> {
        let mut next = self.clone();
        if let Some((_, m)) = next
            .values
            .iter_mut()
            .find(|(n, _)| n.trim().to_lowercase() == name.trim().to_lowercase())
        {
            if !replace {
                return Err("A mixed-font preset with that name already exists; change the name or explicitly update it".into());
            }
            *m = mapping;
        } else {
            next.values.push((name, mapping));
        }
        if !next.valid() {
            return Err(
                "Use unique names up to64 characters, at most32 presets and valid mappings".into(),
            );
        }
        *self = next;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Paint {
    pub color: Option<[u8; 4]>,
    pub border: Option<Border>,
}
#[derive(Clone, Debug)]
pub struct PaintRange {
    pub start: usize,
    pub end: usize,
    pub paint: Paint,
}
