//! Metadata for the validated, static local image-book EPUB profile.
use crate::{
    book_epub,
    book_io::{self, Page},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read},
    sync::atomic::AtomicBool,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub identifier: String,
    pub cover: Option<usize>,
    pub right_binding: bool,
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            title: "Local image book".into(),
            author: String::new(),
            language: "ko".into(),
            identifier: String::new(),
            cover: None,
            right_binding: false,
        }
    }
}
impl Metadata {
    pub fn valid(&self, count: usize) -> bool {
        !self.title.trim().is_empty()
            && self.title.chars().count() <= 256
            && !self.title.chars().any(char::is_control)
            && self.author.chars().count() <= 256
            && !self.author.chars().any(char::is_control)
            && self.language.len() <= 32
            && self.language.split('-').enumerate().all(|(n, s)| {
                !s.is_empty()
                    && s.len() <= 8
                    && s.bytes().all(|b| {
                        if n == 0 {
                            b.is_ascii_alphabetic()
                        } else {
                            b.is_ascii_alphanumeric()
                        }
                    })
            })
            && self.identifier.len() <= 256
            && !self.identifier.chars().any(char::is_control)
            && self.cover.is_none_or(|i| i < count)
    }
}
pub struct Decoded {
    pub pages: Vec<Page>,
    pub metadata: Metadata,
}
fn member(
    z: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
    limit: usize,
) -> Result<String, String> {
    let f = z.by_name(name).map_err(|e| e.to_string())?;
    if f.size() > limit as u64 {
        return Err("EPUB metadata/XHTML size limit".into());
    }
    let mut s = String::new();
    f.take(limit as u64 + 1)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    if s.len() > limit {
        return Err("EPUB metadata size limit".into());
    }
    Ok(s)
}
pub fn decode(
    bytes: &[u8],
    range: &str,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Decoded, String> {
    let pages = book_epub::decode(bytes, range, budget, cancel)?;
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let container = member(&mut z, "META-INF/container.xml", 16384)?;
    let c = roxmltree::Document::parse(&container).map_err(|e| e.to_string())?;
    let opf = c
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .ok_or("Missing EPUB root")?;
    let resolved_opf = book_epub::resolve("", opf)?;
    let opf = resolved_opf.as_str();
    let package = member(&mut z, opf, 4 * 1024 * 1024)?;
    let p = roxmltree::Document::parse(&package).map_err(|e| e.to_string())?;
    let one = |tag: &str, required: bool| -> Result<String, String> {
        let rows = p
            .descendants()
            .filter(|n| n.has_tag_name(("http://purl.org/dc/elements/1.1/", tag)))
            .collect::<Vec<_>>();
        if rows.len() > 1 || (required && rows.len() != 1) {
            return Err(format!("Unsupported EPUB {tag} metadata count"));
        }
        Ok(rows
            .first()
            .and_then(|n| n.text())
            .unwrap_or("")
            .to_string())
    };
    let direction = p
        .descendants()
        .find(|n| n.has_tag_name("spine"))
        .and_then(|n| n.attribute("page-progression-direction"))
        .unwrap_or("ltr");
    if !["ltr", "rtl", "default"].contains(&direction) {
        return Err("Invalid EPUB reading direction".into());
    }
    let mut metadata = Metadata {
        title: one("title", true)?,
        author: one("creator", false)?,
        language: one("language", true)?,
        identifier: one("identifier", true)?,
        cover: None,
        right_binding: direction == "rtl",
    };
    let covers = p
        .descendants()
        .filter(|n| {
            n.has_tag_name("item")
                && n.attribute("properties")
                    .is_some_and(|v| v.split_whitespace().any(|v| v == "cover-image"))
        })
        .collect::<Vec<_>>();
    if covers.len() > 1 {
        return Err("Multiple EPUB cover images are unsupported".into());
    }
    if covers
        .first()
        .is_some_and(|n| !matches!(n.attribute("media-type"), Some("image/png" | "image/jpeg")))
    {
        return Err("EPUB cover must be a declared raster image".into());
    }
    let cover = covers
        .first()
        .map(|n| book_epub::resolve(opf, n.attribute("href").unwrap_or("")))
        .transpose()?;
    let spine = p
        .descendants()
        .filter(|n| n.has_tag_name("itemref"))
        .collect::<Vec<_>>();
    let selected = book_io::selection(range, spine.len())?;
    if let Some(cover) = cover {
        let mut position = None;
        for (index, spine_page) in spine.iter().enumerate() {
            book_io::check_cancel(cancel)?;
            let id = spine_page.attribute("idref").unwrap_or("");
            let page = p
                .descendants()
                .find(|n| n.has_tag_name("item") && n.attribute("id") == Some(id))
                .and_then(|n| n.attribute("href"))
                .ok_or("Missing cover page")?;
            let page = book_epub::resolve(opf, page)?;
            let html = member(&mut z, &page, 1024 * 1024)?;
            let h = roxmltree::Document::parse(&html).map_err(|e| e.to_string())?;
            let image = h
                .descendants()
                .find(|n| n.has_tag_name("img"))
                .and_then(|n| n.attribute("src"))
                .ok_or("Missing cover image")?;
            if book_epub::resolve(&page, image)? == cover && position.replace(index).is_some() {
                return Err("Cover image occurs on multiple spine pages".into());
            }
        }
        let position = position.ok_or("Cover image is not used by a spine page")?;
        metadata.cover = selected.iter().position(|i| *i == position);
    }
    if !metadata.valid(pages.len()) {
        return Err("Invalid or unsupported EPUB metadata".into());
    }
    Ok(Decoded { pages, metadata })
}
pub fn blank_after(pages: &mut Vec<Page>, after: usize, budget: usize) -> Result<(), String> {
    if pages.is_empty() || pages.len() >= 512 || after > pages.len() {
        return Err("Blank insertion position outside1..512 output pages".into());
    }
    let reference = &pages[after.saturating_sub(1)];
    let count = reference.image.pixels.len();
    let used = pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>();
    if used.saturating_add(count * 4) > budget.min(book_io::MAX_BYTES) {
        return Err("Blank page memory limit".into());
    }
    let page = Page {
        image: crate::references::LocalImage {
            width: reference.image.width,
            height: reference.image.height,
            pixels: vec![[255; 4]; count],
        },
        dpi: reference.dpi,
    };
    pages.insert(after, page);
    Ok(())
}
