//! Turn validated static annotation appearances into ordinary page content.
use super::*;
use lopdf::{Dictionary, Object, ObjectId, Stream, dictionary};
fn resolved<'a>(d: &'a lopdf::Document, o: &'a Object) -> Result<&'a Object, String> {
    d.dereference(o).map(|(_, v)| v).map_err(|e| e.to_string())
}
fn dict(d: &lopdf::Document, o: &Object) -> Result<Dictionary, String> {
    resolved(d, o)?
        .as_dict()
        .cloned()
        .map_err(|e| e.to_string())
}
fn numbers(d: &lopdf::Document, o: &Object, count: usize) -> Result<Vec<f64>, String> {
    let v = resolved(d, o)?.as_array().map_err(|e| e.to_string())?;
    if v.len() != count {
        return Err("Invalid annotation rectangle/matrix".into());
    }
    v.iter()
        .map(|o| {
            let n = resolved(d, o)?.as_float().map_err(|e| e.to_string())? as f64;
            if !n.is_finite() || n.abs() > 1e8 {
                return Err("Annotation coordinate limit".into());
            }
            Ok(n)
        })
        .collect()
}
fn resources(d: &lopdf::Document, mut page: ObjectId) -> Result<Dictionary, String> {
    for _ in 0..96 {
        let p = d.get_dictionary(page).map_err(|e| e.to_string())?;
        if let Ok(r) = p.get(b"Resources") {
            return dict(d, r);
        }
        match p.get(b"Parent").and_then(Object::as_reference) {
            Ok(id) => page = id,
            Err(_) => return Ok(Dictionary::new()),
        }
    }
    Err("Cyclic PDF page inheritance".into())
}
pub(super) fn prepare(
    bytes: &[u8],
    range: &str,
    options: &Input,
    c: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let count = crate::book_pdf::validate(bytes)?;
    let selected = book_io::selection(range, count)?;
    let mut d = lopdf::Document::load_mem(bytes).map_err(|e| e.to_string())?;
    let pages = d.get_pages().into_values().collect::<Vec<_>>();
    let mut added_bytes = 0usize;
    for (index, page) in pages.into_iter().enumerate() {
        book_io::check_cancel(c)?;
        let annotations = match d
            .get_dictionary(page)
            .map_err(|e| e.to_string())?
            .get(b"Annots")
        {
            Ok(o) => resolved(&d, o)?
                .as_array()
                .cloned()
                .map_err(|e| e.to_string())?,
            Err(_) => vec![],
        };
        if annotations.len() > 4096 {
            return Err("PDF annotation count limit".into());
        }
        if !options.annotations || !selected.contains(&index) || annotations.is_empty() {
            d.get_dictionary_mut(page)
                .map_err(|e| e.to_string())?
                .remove(b"Annots");
            continue;
        }
        let mut res = resources(&d, page)?;
        let mut xobjects = match res.get(b"XObject") {
            Ok(v) => dict(&d, v)?,
            Err(_) => Dictionary::new(),
        };
        let mut content = String::new();
        if options.annotations && selected.contains(&index) {
            for (n, o) in annotations.iter().enumerate() {
                book_io::check_cancel(c)?;
                let a = dict(&d, o)?;
                let flags = a
                    .get(b"F")
                    .map_or(Ok(0), Object::as_i64)
                    .map_err(|e| e.to_string())?;
                if flags & 35 != 0 {
                    continue;
                }
                if a.has(b"OC") || a.has(b"CA") || a.has(b"BM") {
                    return Err("Annotation optional visibility/opacity/blend is unverified; disable annotation import or export a flattened PDF in its source app".into());
                }
                let ap=a.get(b"AP").map_err(|_|"Annotation has no static appearance; disable annotation import or flatten it in its source app")?;
                let ap = dict(&d, ap)?;
                let mut normal = resolved(
                    &d,
                    ap.get(b"N")
                        .map_err(|_| "Annotation normal appearance is missing")?,
                )?;
                if let Object::Dictionary(states) = normal {
                    let state = a.get(b"AS").and_then(Object::as_name).map_err(
                        |_| "Choose a static normal annotation appearance in the source app",
                    )?;
                    normal = resolved(&d, states.get(state).map_err(|e| e.to_string())?)?;
                }
                let size = normal.as_stream().map_err(|e| e.to_string())?.content.len();
                added_bytes = added_bytes.saturating_add(size);
                if added_bytes > 32 * 1024 * 1024 {
                    return Err("Annotation copied appearances memory limit".into());
                }
                let mut appearance = normal
                    .as_stream()
                    .cloned()
                    .map_err(|_| "Annotation appearance must be a static PDF stream")?;
                if appearance.content.len() > 8 * 1024 * 1024 {
                    return Err("Annotation appearance size limit".into());
                }
                let bbox = numbers(
                    &d,
                    appearance.dict.get(b"BBox").map_err(|e| e.to_string())?,
                    4,
                )?;
                let m = match appearance.dict.get(b"Matrix") {
                    Ok(v) => numbers(&d, v, 6)?,
                    Err(_) => vec![1., 0., 0., 1., 0., 0.],
                };
                let rect = numbers(&d, a.get(b"Rect").map_err(|e| e.to_string())?, 4)?;
                let mut lo = [f64::INFINITY; 2];
                let mut hi = [f64::NEG_INFINITY; 2];
                for (x, y) in [
                    (bbox[0], bbox[1]),
                    (bbox[0], bbox[3]),
                    (bbox[2], bbox[1]),
                    (bbox[2], bbox[3]),
                ] {
                    let q = [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                    for k in 0..2 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
                let w = rect[2] - rect[0];
                let h = rect[3] - rect[1];
                if w <= 0. || h <= 0. || hi[0] - lo[0] < 1e-9 || hi[1] - lo[1] < 1e-9 {
                    return Err("Degenerate annotation appearance bounds".into());
                }
                let sx = w / (hi[0] - lo[0]);
                let sy = h / (hi[1] - lo[1]);
                let tx = rect[0] - sx * lo[0];
                let ty = rect[1] - sy * lo[1];
                appearance.dict.set("Type", "XObject");
                appearance.dict.set("Subtype", "Form");
                let id = d.add_object(appearance);
                let mut name = format!("DrawerAnnotation{n}");
                while xobjects.has(name.as_bytes()) {
                    name.push('x');
                }
                xobjects.set(name.as_bytes(), id);
                content.push_str(&format!(
                    "q {} {} {w} {h} re W n {sx} 0 0 {sy} {tx} {ty} cm /{name} Do Q\n",
                    rect[0], rect[1]
                ));
                if content.len() > 1024 * 1024 {
                    return Err("Annotation content limit".into());
                }
            }
        }
        res.set("XObject", xobjects);
        let p = d.get_dictionary_mut(page).map_err(|e| e.to_string())?;
        p.remove(b"Annots");
        p.set("Resources", res);
        if !content.is_empty() {
            d.add_page_contents(page, content.into_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    d.prune_objects();
    let mut out = vec![];
    d.save_to(&mut out).map_err(|e| e.to_string())?;
    crate::book_pdf::validate(&out)?;
    book_io::check_cancel(c)?;
    Ok(out)
}
/// Owned static stamp fixture used by CPU and Windows-renderer verification.
pub(super) fn fixture() -> Result<Vec<u8>, String> {
    let c = AtomicBool::new(false);
    let bytes = crate::book_pdf::encode(&crate::book_verify::pages(), &c)?;
    let mut d = lopdf::Document::load_mem(&bytes).map_err(|e| e.to_string())?;
    let ap=d.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),1.into(),1.into()],"Resources"=>dictionary!{}},b"1 0 0 rg 0 0 1 1 re f".to_vec()));
    let a=d.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Stamp","Rect"=>vec![0.into(),0.into(),6.into(),9.into()],"AP"=>dictionary!{"N"=>ap}});
    let page = *d.get_pages().get(&1).unwrap();
    d.get_dictionary_mut(page)
        .map_err(|e| e.to_string())?
        .set("Annots", vec![Object::Reference(a)]);
    let mut out = vec![];
    d.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}
