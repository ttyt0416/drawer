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
    book_io::validate(pages, book_io::MAX_BYTES)?;
    let mut d = lopdf::Document::with_version("1.4");
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
        image.compress().map_err(|e| e.to_string())?;
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
    let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>tree});
    d.trailer.set("Root", catalog);
    let mut out = vec![];
    d.save_to(&mut out).map_err(|e| e.to_string())?;
    if out.len() > book_io::MAX_BYTES {
        return Err("PDF output memory limit".into());
    }
    book_io::check_cancel(cancel)?;
    Ok(out)
}
