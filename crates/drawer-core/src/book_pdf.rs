//! Static PDF validation and one-composite-image-per-page PDF output.
use crate::book_io::{self, Page};
use lopdf::{Object, Stream, dictionary};
use std::sync::atomic::AtomicBool;
fn inspect(o: &Object, depth: usize) -> Result<(), String> {
    if depth > 96 {
        return Err("PDF object nesting limit".into());
    }
    let dict = match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => {
            if s.dict.has(b"F") {
                return Err("External PDF streams are unsupported".into());
            }
            Some(&s.dict)
        }
        Object::Array(a) => {
            for v in a {
                inspect(v, depth + 1)?;
            }
            None
        }
        _ => None,
    };
    if let Some(d) = dict {
        for (k, v) in d.iter() {
            if [
                b"Encrypt".as_slice(),
                b"JS",
                b"JavaScript",
                b"OpenAction",
                b"AA",
                b"URI",
                b"URL",
                b"Launch",
                b"GoToR",
                b"SubmitForm",
                b"ImportData",
                b"RichMedia",
                b"EmbeddedFiles",
                b"XFA",
                b"AcroForm",
                b"EF",
            ]
            .contains(&k.as_slice())
                || (k == b"S"
                    && v.as_name().is_ok_and(|n| {
                        [
                            b"JavaScript".as_slice(),
                            b"URI",
                            b"Launch",
                            b"GoToR",
                            b"SubmitForm",
                            b"ImportData",
                            b"Rendition",
                        ]
                        .contains(&n)
                    }))
                || (k == b"Type" && v.as_name().is_ok_and(|n| n == b"Filespec"))
            {
                return Err(
                    "Encrypted, scripted, embedded or remote PDF content is unsupported".into(),
                );
            }
            inspect(v, depth + 1)?;
        }
    }
    Ok(())
}
pub fn validate(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() > 32 * 1024 * 1024 || !bytes.starts_with(b"%PDF-") {
        return Err("PDF input limit 32 MiB or invalid PDF header".into());
    }
    let d = lopdf::Document::load_mem_with_options(
        bytes,
        lopdf::LoadOptions {
            strict: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Cannot read static PDF (encrypted/unsupported): {e}"))?;
    if d.is_encrypted() || d.encryption_state.is_some() || d.objects.len() > 100_000 {
        return Err("Encrypted PDF or PDF object limit".into());
    }
    inspect(&Object::Dictionary(d.trailer.clone()), 0)?;
    for o in d.objects.values() {
        inspect(o, 0)?;
    }
    let count = d.get_pages().len();
    if count == 0 || count > book_io::MAX_PAGES {
        return Err("PDF page count must be 1..512".into());
    }
    Ok(count)
}
pub fn encode(pages: &[Page], cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    encode_options(pages, &crate::book_options::Output::default(), cancel)
}
pub fn encode_options(
    pages: &[Page],
    options: &crate::book_options::Output,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    encode_options_budget(pages, options, book_io::MAX_BYTES.saturating_mul(4), cancel)
}
pub fn encode_options_budget(
    pages: &[Page],
    options: &crate::book_options::Output,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    book_io::validate(pages, book_io::MAX_BYTES)?;
    let raw = pages.iter().map(|p| p.image.memory_bytes()).sum::<usize>();
    let jpeg = if options.compression.quality().is_some() {
        pages
            .iter()
            .map(|p| p.image.memory_bytes())
            .max()
            .unwrap_or(0)
            .saturating_mul(9)
    } else {
        0
    };
    if raw
        .saturating_mul(4)
        .saturating_add(jpeg)
        .saturating_add(1024 * 1024)
        > budget
    {
        return Err("PDF encoding workspace memory reserve; choose fewer/smaller pages".into());
    }
    if !options.metadata.valid() {
        return Err("PDF metadata limit".into());
    }
    let mut d = lopdf::Document::with_version("1.5");
    d.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let tree = d.new_object_id();
    let mut kids = vec![];
    for p in pages {
        book_io::check_cancel(cancel)?;
        let w = p.image.width;
        let h = p.image.height;
        let rgb = p
            .image
            .pixels
            .iter()
            .flat_map(|p| p[..3].iter().copied())
            .collect();
        let alpha = p.image.pixels.iter().map(|p| p[3]).collect();
        let mut mask = Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>w,"Height"=>h,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8},
            alpha,
        );
        mask.compress().map_err(|e| e.to_string())?;
        let mask = d.add_object(mask);
        let mut image = Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>w,"Height"=>h,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"SMask"=>mask},
            rgb,
        );
        if let Some(q) = options.compression.quality() {
            let jpeg = crate::jpeg_output::encode(
                crate::jpeg_output::Image {
                    rgb: &image.content,
                    width: w,
                    height: h,
                    dpi: p.dpi,
                    icc: None,
                },
                crate::jpeg_output::Settings {
                    luma: q,
                    chroma: q,
                    sampling: crate::jpeg_output::Sampling::S444,
                },
                book_io::MAX_BYTES,
                cancel,
            )?;
            image.set_content(jpeg);
            image.dict.set("Filter", "DCTDecode");
        } else {
            image.compress().map_err(|e| e.to_string())?;
        }
        let image = d.add_object(image);
        let pw = w as f64 * 72. / p.dpi;
        let ph = h as f64 * 72. / p.dpi;
        let content = d.add_object(Stream::new(
            dictionary! {},
            format!("q {pw:.8} 0 0 {ph:.8} 0 0 cm /PageImage Do Q").into_bytes(),
        ));
        let page=d.add_object(dictionary!{"Type"=>"Page","Parent"=>tree,"MediaBox"=>vec![0.into(),0.into(),(pw as f32).into(),(ph as f32).into()],"Contents"=>content,"Resources"=>dictionary!{"XObject"=>dictionary!{"PageImage"=>image}}});
        kids.push(Object::Reference(page));
    }
    d.objects.insert(
        tree,
        Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>pages.len() as i64}),
    );
    let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>tree,
    "PageLayout"=>options.layout.name(options.right_binding),
    "Lang"=>lopdf::text_string(&options.metadata.language),
    "ViewerPreferences"=>dictionary!{"Direction"=>if options.right_binding{"R2L"}else{"L2R"}}});
    let info=d.add_object(dictionary!{"Title"=>lopdf::text_string(&options.metadata.title),"Author"=>lopdf::text_string(&options.metadata.author),"Creator"=>lopdf::text_string("Drawer local raster exchange")});
    d.trailer.set("Info", info);
    d.trailer.set("Root", catalog);
    let mut out = vec![];
    d.save_to(&mut out).map_err(|e| e.to_string())?;
    if out.len() > book_io::MAX_BYTES {
        return Err("PDF output memory limit".into());
    }
    book_io::check_cancel(cancel)?;
    Ok(out)
}
