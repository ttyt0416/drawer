use crate::workspaces::Snapshot;
use serde::{Deserialize, Serialize};
pub const CONTENT: &str = include_str!("../../../docs/HELP.md");
pub const NOTICES: &str = include_str!("../../../docs/licenses/third-party-notices.txt");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn supports_korean(bytes: &[u8]) -> bool {
    ttf_parser::Face::parse(bytes, 0)
        .ok()
        .is_some_and(|f| f.glyph_index('한').is_some() && f.glyph_index('글').is_some())
}
#[derive(Clone, Copy, Debug)]
pub struct Topic {
    pub id: &'static str,
    pub title: &'static str,
    pub keywords: &'static str,
    pub body: &'static str,
}
pub fn topics() -> Vec<Topic> {
    CONTENT
        .split("\n## ")
        .filter_map(|section| {
            let (header, body) = section.split_once('\n')?;
            let mut fields = header.split(" | ");
            Some(Topic {
                id: fields.next()?,
                title: fields.next()?,
                keywords: fields.next()?,
                body: body.trim(),
            })
        })
        .collect()
}
pub fn find(id: &str) -> Option<Topic> {
    topics().into_iter().find(|t| t.id == id)
}
pub fn link(line: &str) -> Option<(&str, &str)> {
    let (label, target) = line.trim().strip_prefix('[')?.split_once("](local:")?;
    Some((label, target.strip_suffix(')')?))
}
pub fn search(query: &str, settings: &Snapshot) -> Vec<Topic> {
    let q = query.trim().to_lowercase();
    let terms = q.split_whitespace().collect::<Vec<_>>();
    let bindings = settings
        .bindings
        .iter()
        .map(|b| format!("{} {}", b.label(), settings.actions.label(&b.command)))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    static NOTICE_SEARCH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    topics()
        .into_iter()
        .filter(|t| {
            if terms.is_empty() {
                return true;
            }
            let mut text = format!("{} {} {}", t.title, t.keywords, t.body).to_lowercase();
            if t.id == "shortcuts" {
                text.push_str(&bindings);
            }
            if terms.iter().all(|term| text.contains(term)) {
                return true;
            }
            t.id == "licenses"
                && terms.iter().all(|term| {
                    NOTICE_SEARCH
                        .get_or_init(|| NOTICES.to_lowercase())
                        .contains(term)
                })
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub visible: bool,
    pub topic: String,
    pub query: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            visible: false,
            topic: "welcome".into(),
            query: String::new(),
        }
    }
}
impl Preferences {
    pub fn valid(&self) -> bool {
        self.version == 1
            && !self.topic.is_empty()
            && self.topic.len() <= 128
            && self.query.len() <= 1024
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn korean_case_empty_missing_dynamic_shortcuts_and_local_links() {
        let mut s = Snapshot::default();
        assert_eq!(search("", &s).len(), topics().len());
        assert!(search("없는도움말987654321", &s).is_empty());
        assert!(search("작업공간", &s).iter().any(|t| t.id == "workspace"));
        assert!(search("CTRL+S", &s).iter().any(|t| t.id == "shortcuts"));
        s.bindings[5].key = "F2".into();
        assert!(search("F2", &s).iter().any(|t| t.id == "shortcuts"));
        for topic in topics() {
            for line in topic.body.lines().filter(|l| l.starts_with('[')) {
                let (_, target) = link(line).expect("Only local links are allowed");
                assert!(find(target).is_some(), "{target}");
            }
        }
        assert!(find("missing-topic").is_none());
        assert!(NOTICES.len() > 100_000);
        assert!(!VERSION.is_empty());
    }
}
