//! Local PDF interchange options. Viewer metadata never changes page pixels.
use crate::{book_io, storage::UserStorage};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    Zip,
    Highest,
    High,
    Medium,
    Low,
    Lowest,
}
impl Compression {
    pub const ALL: [Self; 6] = [
        Self::Zip,
        Self::Highest,
        Self::High,
        Self::Medium,
        Self::Low,
        Self::Lowest,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "ZIP (lossless)",
            Self::Highest => "JPEG (Highest)",
            Self::High => "JPEG (High)",
            Self::Medium => "JPEG (Medium)",
            Self::Low => "JPEG (Low)",
            Self::Lowest => "JPEG (Lowest)",
        }
    }
    pub fn quality(self) -> Option<u8> {
        match self {
            Self::Zip => None,
            Self::Highest => Some(100),
            Self::High => Some(90),
            Self::Medium => Some(75),
            Self::Low => Some(50),
            Self::Lowest => Some(25),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Single,
    Continuous,
    TwoUp,
    TwoUpContinuous,
}
impl Layout {
    pub const ALL: [Self; 4] = [
        Self::Single,
        Self::Continuous,
        Self::TwoUp,
        Self::TwoUpContinuous,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "Single Page",
            Self::Continuous => "Single Page Continuous",
            Self::TwoUp => "Two-Up",
            Self::TwoUpContinuous => "Two-Up Continuous",
        }
    }
    pub fn name(self, right: bool) -> &'static str {
        match (self, right) {
            (Self::Single, _) => "SinglePage",
            (Self::Continuous, _) => "OneColumn",
            (Self::TwoUp, false) => "TwoPageLeft",
            (Self::TwoUp, true) => "TwoPageRight",
            (Self::TwoUpContinuous, false) => "TwoColumnLeft",
            (Self::TwoUpContinuous, true) => "TwoColumnRight",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub right_binding: bool,
    pub from_right: bool,
    pub annotations: bool,
}
impl Input {
    pub fn order(&self, count: usize, range: &str) -> Result<Vec<usize>, String> {
        let mut a = book_io::selection(range, count)?;
        if self.from_right {
            a.reverse();
        }
        Ok(a)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub language: String,
}
impl Metadata {
    pub fn valid(&self) -> bool {
        [&self.title, &self.author, &self.language]
            .iter()
            .all(|s| s.len() <= 4096 && !s.contains('\0'))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    pub compression: Compression,
    pub layout: Layout,
    pub right_binding: bool,
    pub metadata: Metadata,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub input: Input,
    pub output: Output,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            input: Default::default(),
            output: Default::default(),
        }
    }
}
impl Settings {
    pub fn load(s: &UserStorage) -> Result<Self, String> {
        let p = s.root().join("settings/book-options.json");
        let b = match std::fs::read(p) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        if b.len() > 65536 {
            return Err("PDF settings size limit; original retained".into());
        }
        let v: Self = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        if v.version != 1 || !v.output.metadata.valid() {
            return Err("Invalid PDF settings; original retained".into());
        }
        Ok(v)
    }
    pub fn save(&self, s: &UserStorage) -> Result<(), String> {
        use std::io::Write;
        if self.version != 1 || !self.output.metadata.valid() {
            return Err("Invalid PDF settings".into());
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let p = s.root().join("settings/book-options.json");
        let tmp = p.with_extension("json.write.tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        let r = (|| {
            f.write_all(&bytes).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
        })();
        let _ = std::fs::remove_file(tmp);
        r
    }
}
pub fn metadata(bytes: &[u8]) -> Result<Metadata, String> {
    crate::book_pdf::validate(bytes)?;
    let d = lopdf::Document::load_mem(bytes).map_err(|e| e.to_string())?;
    let mut m = Metadata::default();
    if let Ok(info) = d
        .trailer
        .get(b"Info")
        .and_then(lopdf::Object::as_reference)
        .and_then(|id| d.get_dictionary(id))
    {
        for (key, out) in [
            (b"Title".as_slice(), &mut m.title),
            (b"Author", &mut m.author),
        ] {
            if let Ok(o) = info.get(key) {
                *out = lopdf::decode_text_string(o).map_err(|e| e.to_string())?;
            }
        }
    }
    if let Ok(c) = d.catalog()
        && let Ok(o) = c.get(b"Lang")
    {
        m.language = lopdf::decode_text_string(o).map_err(|e| e.to_string())?;
    }
    if !m.valid() {
        return Err("PDF metadata limit".into());
    }
    Ok(m)
}
#[path = "pdf_annotations.rs"]
mod annotations;
pub fn prepare_import(
    bytes: &[u8],
    range: &str,
    options: &Input,
    c: &AtomicBool,
) -> Result<Vec<u8>, String> {
    annotations::prepare(bytes, range, options, c)
}
#[path = "book_options_verify.rs"]
pub mod verify;
