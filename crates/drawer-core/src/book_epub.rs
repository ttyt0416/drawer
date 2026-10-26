//! Fixed-layout, full-page local PNG/JPEG EPUB profile; no browser or extraction.
use crate::book_io::{self, Page};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
    sync::atomic::AtomicBool,
};
const XHTML: &str = "http://www.w3.org/1999/xhtml";
const STYLE: &str =
    "html,body{margin:0;padding:0;width:100%;height:100%}img{width:100%;height:100%}";
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}
fn resolve(base: &str, href: &str) -> Result<String, String> {
    let mut decoded = vec![];
    let mut i = 0;
    let b = href.as_bytes();
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return Err("Invalid EPUB URL escape".into());
            }
            decoded.push(
                u8::from_str_radix(
                    std::str::from_utf8(&b[i + 1..i + 3]).map_err(|_| "Invalid EPUB URL escape")?,
                    16,
                )
                .map_err(|_| "Invalid EPUB URL escape")?,
            );
            i += 3;
        } else {
            decoded.push(b[i]);
            i += 1;
        }
    }
    let href = std::str::from_utf8(&decoded).map_err(|_| "EPUB URL must be UTF-8")?;
    if href.is_empty()
        || href.starts_with('/')
        || href.contains([':', '\\', '?', '#'])
        || href.chars().any(char::is_control)
    {
        return Err("Only local EPUB resource paths are supported".into());
    }
    let mut parts: Vec<&str> = base
        .rsplit_once('/')
        .map_or(vec![], |(p, _)| p.split('/').collect());
    for p in href.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or("EPUB path escapes archive")?;
            }
            _ => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err("Empty EPUB resource".into());
    }
    Ok(parts.join("/"))
}
fn text<'a>(files: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<&'a str, String> {
    let bytes = files
        .get(name)
        .ok_or_else(|| format!("Missing EPUB resource: {name}"))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("EPUB XML size limit".into());
    }
    std::str::from_utf8(bytes).map_err(|_| "EPUB XML must be UTF-8".into())
}
fn parse(s: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse_with_options(
        s,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Invalid EPUB XML: {e}"))
}
fn safe_html(d: &roxmltree::Document<'_>, base: &str) -> Result<(), String> {
    for n in d.descendants().filter(|n| n.is_element()) {
        if n.tag_name().namespace() != Some(XHTML)
            || ![
                "html", "head", "title", "meta", "style", "body", "img", "nav", "ol", "li", "a",
            ]
            .contains(&n.tag_name().name())
        {
            return Err(
                "Unsupported EPUB content: only static full-page images are imported".into(),
            );
        }
        if n.tag_name().name() == "style" && n.text().unwrap_or("").trim() != STYLE {
            return Err("Unsupported EPUB page CSS; use a full-page image layout".into());
        }
        for a in n.attributes() {
            let key = a.name().to_ascii_lowercase();
            if key.starts_with("on")
                || ["srcset", "srcdoc", "http-equiv"].contains(&key.as_str())
                || (key == "style"
                    && !["width:100%;height:100%", "margin:0;padding:0"].contains(&a.value()))
            {
                return Err("Scripted or unsupported EPUB styling is not imported".into());
            }
            if ["src", "href"].contains(&key.as_str()) {
                let value = a.value().split('#').next().unwrap();
                resolve(base, value)?;
            }
        }
    }
    Ok(())
}
pub fn decode(
    bytes: &[u8],
    range: &str,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Page>, String> {
    if bytes.len() > budget.min(book_io::MAX_BYTES) / 2 {
        return Err("EPUB compressed input size limit".into());
    }
    let mut z = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("Invalid/encrypted EPUB: {e}"))?;
    if z.is_empty() || z.len() > 4096 {
        return Err("EPUB resource count limit".into());
    }
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for i in 0..z.len() {
        book_io::check_cancel(cancel)?;
        let f = z
            .by_index(i)
            .map_err(|e| format!("Encrypted/unsupported EPUB entry: {e}"))?;
        let name = f.name().to_string();
        if name.starts_with('/')
            || name.contains(['\\', ':'])
            || name.split('/').any(|p| p == "..")
            || name.chars().any(char::is_control)
            || f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("Unsafe EPUB resource path or link".into());
        }
        if i == 0 && name != "mimetype" {
            return Err("EPUB mimetype must be first".into());
        }
        if f.is_dir() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if ["meta-inf/encryption.xml", "meta-inf/rights.xml"].contains(&lower.as_str())
            || lower.ends_with(".js")
        {
            return Err("DRM, encrypted or scripted EPUB is unsupported".into());
        }
        if i == 0 && (name != "mimetype" || f.compression() != zip::CompressionMethod::Stored) {
            return Err("EPUB requires an uncompressed first mimetype entry".into());
        }
        let cap = budget.min(book_io::MAX_BYTES) / 2 - total;
        if f.size() > cap as u64 {
            return Err("EPUB expanded resource memory limit".into());
        }
        let mut b = vec![];
        f.take(cap as u64 + 1)
            .read_to_end(&mut b)
            .map_err(|e| e.to_string())?;
        total = total.saturating_add(b.len());
        if total > budget.min(book_io::MAX_BYTES) / 2 || files.insert(name, b).is_some() {
            return Err("EPUB resource limit or duplicate filename".into());
        }
    }
    if files.get("mimetype").map(Vec::as_slice) != Some(b"application/epub+zip") {
        return Err("Not an EPUB container".into());
    }
    let container = parse(text(&files, "META-INF/container.xml")?)?;
    let roots: Vec<_> = container
        .descendants()
        .filter(|n| n.has_tag_name("rootfile"))
        .collect();
    if roots.len() != 1 {
        return Err("Choose an EPUB with one rendition".into());
    }
    let opf = resolve(
        "",
        roots[0]
            .attribute("full-path")
            .ok_or("Missing EPUB package path")?,
    )?;
    let package = parse(text(&files, &opf)?)?;
    let layouts: Vec<_> = package
        .descendants()
        .filter(|n| n.has_tag_name("meta") && n.attribute("property") == Some("rendition:layout"))
        .collect();
    if layouts.len() != 1
        || layouts[0]
            .text()
            .is_none_or(|s| s.trim() != "pre-paginated")
    {
        return Err("Reflowable EPUB is unsupported; use fixed-layout image pages".into());
    }
    let mut items = BTreeMap::new();
    for n in package.descendants().filter(|n| n.has_tag_name("item")) {
        let id = n.attribute("id").ok_or("Missing EPUB item ID")?;
        let path = resolve(&opf, n.attribute("href").ok_or("Missing EPUB item path")?)?;
        let media = n.attribute("media-type").ok_or("Missing EPUB media type")?;
        if !["application/xhtml+xml", "image/png", "image/jpeg"].contains(&media)
            || n.attribute("properties").is_some_and(|s| {
                s.split_whitespace()
                    .any(|p| p != "nav" && p != "cover-image")
            })
        {
            return Err("Unsupported EPUB media, script or remote resource".into());
        }
        if !files.contains_key(&path)
            || items
                .insert(
                    id,
                    (
                        path.clone(),
                        media,
                        n.attribute("properties") == Some("nav"),
                    ),
                )
                .is_some()
        {
            return Err("Missing EPUB item or duplicate ID".into());
        }
        if media == "application/xhtml+xml" {
            safe_html(&parse(text(&files, &path)?)?, &path)?;
        }
    }
    for name in files.keys() {
        if !["mimetype", "META-INF/container.xml", opf.as_str()].contains(&name.as_str())
            && !items.values().any(|i| &i.0 == name)
        {
            return Err(
                "Unlisted EPUB resources are unsupported; use a self-contained image book".into(),
            );
        }
    }
    if package.descendants().any(|n| {
        n.has_tag_name("itemref")
            && n.attribute("properties")
                .is_some_and(|s| s.contains("reflowable"))
    }) {
        return Err("Reflowable EPUB spine override is unsupported".into());
    }
    if !items.values().any(|i| i.2) {
        return Err("Missing EPUB navigation document".into());
    }
    let spine: Vec<_> = package
        .descendants()
        .filter(|n| n.has_tag_name("itemref"))
        .collect();
    let chosen = book_io::selection(range, spine.len())?;
    let mut pages = vec![];
    let mut used = total + bytes.len();
    for index in chosen {
        book_io::check_cancel(cancel)?;
        let n = spine[index];
        if n.attribute("properties")
            .is_some_and(|s| s.contains("reflowable"))
            || n.attribute("linear") == Some("no")
        {
            return Err("Unsupported EPUB spine override".into());
        }
        let (path, media, nav) = items
            .get(n.attribute("idref").ok_or("Missing spine reference")?)
            .ok_or("Unknown spine reference")?;
        if *media != "application/xhtml+xml" || *nav {
            return Err("EPUB spine must contain image XHTML pages".into());
        }
        let html = parse(text(&files, path)?)?;
        let imgs: Vec<_> = html
            .descendants()
            .filter(|n| n.has_tag_name((XHTML, "img")))
            .collect();
        if imgs.len() != 1
            || html.descendants().any(|n| {
                n.is_text()
                    && n.text().is_some_and(|s| !s.trim().is_empty())
                    && !n.ancestors().any(|a| a.has_tag_name((XHTML, "head")))
            })
        {
            return Err("Only one full-page image per EPUB page is supported".into());
        }
        let src = resolve(path, imgs[0].attribute("src").ok_or("Missing page image")?)?;
        if !items
            .values()
            .any(|i| i.0 == src && ["image/png", "image/jpeg"].contains(&i.1))
        {
            return Err("Unlisted EPUB image".into());
        }
        let image = book_io::decode(
            files.get(&src).ok_or("Missing page image bytes")?,
            budget.min(book_io::MAX_BYTES).saturating_sub(used),
        )?;
        let viewport = html
            .descendants()
            .find(|n| n.has_tag_name((XHTML, "meta")) && n.attribute("name") == Some("viewport"))
            .and_then(|n| n.attribute("content"))
            .ok_or("Missing fixed page viewport")?;
        let size = viewport
            .split(',')
            .map(|s| s.trim().split_once('=').ok_or("Invalid page viewport"))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        if size.get("width").and_then(|s| s.parse::<u32>().ok()) != Some(image.width)
            || size.get("height").and_then(|s| s.parse::<u32>().ok()) != Some(image.height)
        {
            return Err(
                "EPUB image must fill its page viewport without scaling or cropping".into(),
            );
        }
        let dpi = html
            .descendants()
            .find(|n| n.has_tag_name((XHTML, "meta")) && n.attribute("name") == Some("drawer-dpi"))
            .map(|n| {
                n.attribute("content")
                    .unwrap_or("")
                    .parse::<f64>()
                    .map_err(|_| "Invalid page DPI")
            })
            .transpose()?
            .unwrap_or(96.);
        used = used.saturating_add(image.memory_bytes());
        pages.push(Page { image, dpi });
    }
    book_io::validate(
        &pages,
        budget
            .min(book_io::MAX_BYTES)
            .saturating_sub(total + bytes.len()),
    )?;
    book_io::check_cancel(cancel)?;
    Ok(pages)
}
pub fn encode(pages: &[Page], title: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    book_io::validate(pages, book_io::MAX_BYTES)?;
    if title.trim().is_empty() || title.len() > 4096 || title.chars().any(char::is_control) {
        return Err("Choose a book title".into());
    }
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for p in pages {
        hash.update(p.image.width.to_le_bytes());
        hash.update(p.image.height.to_le_bytes());
        hash.update(p.dpi.to_le_bytes());
        for pixel in &p.image.pixels {
            hash.update(pixel);
        }
    }
    let identifier = format!("urn:sha256:{:x}", hash.finalize());
    let mut zip = zip::ZipWriter::new(Cursor::new(vec![]));
    let stored = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o644);
    let deflated = stored.compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, bytes: &[u8], compressed: bool| -> Result<(), String> {
        zip.start_file(name, if compressed { deflated } else { stored })
            .map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())
    };
    add("mimetype", b"application/epub+zip", false)?;
    add("META-INF/container.xml",br#"<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,true)?;
    let mut manifest = String::from(
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>",
    );
    let mut spine = String::new();
    let mut links = String::new();
    for (i, p) in pages.iter().enumerate() {
        book_io::check_cancel(cancel)?;
        let num = i + 1;
        let png = format!("images/page-{num:04}.png");
        let xhtml = format!("page-{num:04}.xhtml");
        let raw: Vec<_> = p.image.pixels.iter().flatten().copied().collect();
        let mut encoded = vec![];
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(
                &raw,
                p.image.width,
                p.image.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        add(&format!("EPUB/{png}"), &encoded, false)?;
        let html = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"{XHTML}\"><head><title>Page {num}</title><meta name=\"viewport\" content=\"width={},height={}\"/><meta name=\"drawer-dpi\" content=\"{}\"/><style>{STYLE}</style></head><body><img src=\"{png}\" alt=\"Page {num}\"/></body></html>",
            p.image.width, p.image.height, p.dpi
        );
        add(&format!("EPUB/{xhtml}"), html.as_bytes(), true)?;
        manifest.push_str(&format!("<item id=\"page{num}\" href=\"{xhtml}\" media-type=\"application/xhtml+xml\"/><item id=\"image{num}\" href=\"{png}\" media-type=\"image/png\"/>"));
        spine.push_str(&format!("<itemref idref=\"page{num}\"/>"));
        links.push_str(&format!("<li><a href=\"{xhtml}\">Page {num}</a></li>"));
    }
    let nav = format!(
        "<?xml version=\"1.0\"?><html xmlns=\"{XHTML}\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body><nav epub:type=\"toc\"><ol>{links}</ol></nav></body></html>"
    );
    add("EPUB/nav.xhtml", nav.as_bytes(), true)?;
    let package = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"book-id\">{identifier}</dc:identifier><dc:title>{}</dc:title><dc:language>ko</dc:language><meta property=\"dcterms:modified\">1980-01-01T00:00:00Z</meta><meta property=\"rendition:layout\">pre-paginated</meta><meta property=\"rendition:spread\">none</meta></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>",
        xml(title)
    );
    add("EPUB/package.opf", package.as_bytes(), true)?;
    let out = zip.finish().map_err(|e| e.to_string())?.into_inner();
    if out.len() > book_io::MAX_BYTES {
        return Err("EPUB output memory limit".into());
    }
    book_io::check_cancel(cancel)?;
    Ok(out)
}
