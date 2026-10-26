//! Local, per-document viewing preferences are deliberately outside document Undo.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Viewport {
    pub enabled: bool,
    pub width: u32,
    pub height: u32,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 800,
            height: 1280,
        }
    }
}
impl Viewport {
    pub fn valid(self) -> bool {
        (1..=32768).contains(&self.width) && (1..=32768).contains(&self.height)
    }
    pub fn region(self, doc_width: u32, doc_height: u32, top: f64) -> crate::scene::Rect {
        let h = doc_width as f64 * self.height as f64 / self.width as f64;
        let y = top.clamp(0., (doc_height as f64 - h).max(0.));
        crate::scene::Rect {
            min: [0., y],
            max: [doc_width as f64, (y + h).min(doc_height as f64)],
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Local {
    pub pages: Vec<(std::path::PathBuf, Viewport)>,
    pub show_breaks: bool,
    pub vertical_list: bool,
    pub append: bool,
    pub inherit: bool,
}
impl Local {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn valid(&self) -> bool {
        self.pages.len() <= 128
            && self.pages.iter().enumerate().all(|(n, (p, v))| {
                crate::font_library::local(p)
                    && v.valid()
                    && !self.pages[..n].iter().any(|(q, _)| q == p)
            })
    }
    pub fn get(&self, path: Option<&std::path::Path>) -> Viewport {
        path.and_then(|p| self.pages.iter().find(|(q, _)| q == p).map(|(_, v)| *v))
            .unwrap_or_default()
    }
    pub fn set(&mut self, path: std::path::PathBuf, view: Viewport) {
        if let Some((_, v)) = self.pages.iter_mut().find(|(p, _)| *p == path) {
            *v = view;
        } else {
            if self.pages.len() >= 128 {
                self.pages.remove(0);
            }
            self.pages.push((path, view));
        }
    }
}
