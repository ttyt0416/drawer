use super::*;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
};
fn u16w(out: &mut Vec<u8>, n: u16) {
    out.extend(n.to_be_bytes());
}
fn u32w(out: &mut Vec<u8>, n: u32) {
    out.extend(n.to_be_bytes());
}
fn length(out: &mut Vec<u8>, n: usize, f: Format) -> Result<(), String> {
    match f {
        Format::Psd => u32w(
            out,
            u32::try_from(n).map_err(|_| "PSD section exceeds 32-bit length; use PSB")?,
        ),
        Format::Psb => out.extend((n as u64).to_be_bytes()),
    }
    Ok(())
}
fn block(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) {
    out.extend(b"8BIM");
    out.extend(key);
    u32w(out, data.len() as u32);
    out.extend(data);
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
}
fn srgb_icc() -> Result<Vec<u8>, String> {
    let mut bytes = moxcms::ColorProfile::new_srgb()
        .encode()
        .map_err(|e| e.to_string())?;
    for (slot, n) in bytes[24..36]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip([2000u16, 1, 1, 0, 0, 0])
    {
        slot.copy_from_slice(&n.to_be_bytes());
    }
    Ok(bytes)
}
fn resource(out: &mut Vec<u8>, id: u16, data: &[u8]) {
    out.extend(b"8BIM");
    u16w(out, id);
    out.extend([0, 0]);
    u32w(out, data.len() as u32);
    out.extend(data);
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
}
pub(super) fn encode(
    image: &Image,
    f: Format,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    encode_with_preview(image, f, true, budget, cancel)
}
pub(super) fn encode_with_preview(
    image: &Image,
    f: Format,
    merged_preview: bool,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if image.width == 0
        || image.height == 0
        || image.width > if f == Format::Psd { 30000 } else { 32768 }
        || image.height > if f == Format::Psd { 30000 } else { 32768 }
        || image.layers.is_empty()
        || image.layers.len() > 768
        || image.merged.len() != image.width as usize * image.height as usize
    {
        return Err("Invalid PSD/PSB dimensions, layers or merged image".into());
    }
    let samples = image
        .layers
        .iter()
        .try_fold(image.merged.len(), |sum, l| {
            sum.checked_add(l.pixels.len())
                .and_then(|s| s.checked_add(l.mask.as_ref().map_or(0, |m| m.pixels.len())))
        })
        .ok_or("PSD sample count overflow")?;
    if samples.saturating_mul(12) > budget {
        return Err("PSD serialization memory reserve".into());
    }
    let mut records = vec![];
    let mut channels = vec![];
    // PSD records and their channel payloads run from the bottom of the stack upward.
    for layer in image.layers.iter().rev() {
        canceled(cancel)?;
        let (w, h) = area(layer.bounds)?;
        if layer.section == 0 && layer.pixels.len() != w * h {
            return Err("PSD layer raster size mismatch".into());
        }
        for n in layer.bounds {
            records.extend(n.to_be_bytes());
        }
        let count = if layer.section == 0 { 4 } else { 0 } + usize::from(layer.mask.is_some());
        u16w(&mut records, count as u16);
        if layer.section == 0 {
            for id in [0i16, 1, 2, -1] {
                records.extend(id.to_be_bytes());
                length(&mut records, 2 + layer.pixels.len(), f)?;
                u16w(&mut channels, 0);
                for p in &layer.pixels {
                    channels.push(p[if id < 0 { 3 } else { id as usize }]);
                }
            }
        }
        if let Some(mask) = &layer.mask {
            let (w, h) = area(mask.bounds)?;
            if mask.pixels.len() != w * h {
                return Err("PSD mask size mismatch".into());
            }
            records.extend((-2i16).to_be_bytes());
            length(&mut records, 2 + mask.pixels.len(), f)?;
            u16w(&mut channels, 0);
            channels.extend(&mask.pixels);
        }
        records.extend(b"8BIM");
        records.extend(layer.blend);
        records.extend([
            layer.opacity,
            layer.clipped as u8,
            2 * layer.hidden as u8 + layer.alpha_lock as u8,
            0,
        ]);
        let mut extra = vec![];
        if let Some(mask) = &layer.mask {
            u32w(&mut extra, 20);
            for n in mask.bounds {
                extra.extend(n.to_be_bytes());
            }
            extra.extend([mask.default, mask.flags, 0, 0]);
        } else {
            u32w(&mut extra, 0);
        }
        u32w(&mut extra, 0);
        extra.extend([0, 0, 0, 0]);
        let name = layer.name.encode_utf16().collect::<Vec<_>>();
        let mut unicode = vec![];
        u32w(&mut unicode, name.len() as u32);
        for n in name {
            u16w(&mut unicode, n);
        }
        block(&mut extra, b"luni", &unicode);
        if layer.section != 0 {
            let mut section = vec![];
            u32w(&mut section, layer.section);
            section.extend(b"8BIM");
            section.extend(layer.blend);
            block(&mut extra, b"lsct", &section);
        }
        u32w(&mut records, extra.len() as u32);
        records.extend(extra);
    }
    let mut info = vec![];
    info.extend((-(image.layers.len() as i16)).to_be_bytes());
    info.extend(records);
    info.extend(channels);
    if info.len() % 2 != 0 {
        info.push(0);
    }
    let mut layers = vec![];
    length(&mut layers, info.len(), f)?;
    layers.extend(info);
    u32w(&mut layers, 0);
    let mut resources = vec![];
    let mut dpi = vec![];
    for _ in 0..2 {
        u32w(
            &mut dpi,
            (image.dpi.clamp(1., 9600.) * 65536.).round() as u32,
        );
        u16w(&mut dpi, 1);
        u16w(&mut dpi, 1);
    }
    resource(&mut resources, 1005, &dpi);
    resource(&mut resources, 1039, &srgb_icc()?);
    let mut version = vec![];
    u32w(&mut version, 1);
    version.push(merged_preview as u8);
    for name in ["Drawer", "Drawer"] {
        let words = name.encode_utf16().collect::<Vec<_>>();
        u32w(&mut version, words.len() as u32);
        for w in words {
            u16w(&mut version, w);
        }
    }
    u32w(&mut version, 1);
    resource(&mut resources, 1057, &version);
    let mut out = b"8BPS".to_vec();
    u16w(&mut out, if f == Format::Psd { 1 } else { 2 });
    out.extend([0; 6]);
    u16w(&mut out, 4);
    u32w(&mut out, image.height);
    u32w(&mut out, image.width);
    u16w(&mut out, 8);
    u16w(&mut out, 3);
    u32w(&mut out, 0);
    u32w(&mut out, resources.len() as u32);
    out.extend(resources);
    length(&mut out, layers.len(), f)?;
    out.extend(layers);
    u16w(&mut out, 0);
    for c in 0..4 {
        canceled(cancel)?;
        for p in &image.merged {
            // Merged RGB is white-matted when its extra channel stores transparency.
            out.push(if !merged_preview {
                if c == 3 { 0 } else { 255 }
            } else if c == 3 {
                p[c]
            } else {
                ((p[c] as u32 * p[3] as u32 + 255 * (255 - p[3] as u32) + 127) / 255) as u8
            });
        }
    }
    if out.len() > budget / 2 {
        return Err("PSD file exceeds write memory limit".into());
    }
    Ok(out)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn write(
    image: &Image,
    d: &Document,
    path: &Path,
    format: Format,
    merged_preview: bool,
    replace: bool,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<(), String> {
    crate::exchange_guidance::reject_private(path)?;
    let expected = if format == Format::Psd { "psd" } else { "psb" };
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(expected))
    {
        return Err(format!("Choose a .{expected} destination"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !crate::font_library::local(&parent) {
        return Err("Choose a local PSD/PSB folder".into());
    }
    let path = parent.join(path.file_name().ok_or("Missing PSD filename")?);
    let check = || -> Result<(), String> {
        let exists = match fs::symlink_metadata(&path) {
            Ok(m) => {
                if m.file_type().is_symlink() || !m.is_file() {
                    return Err("Choose a regular PSD destination".into());
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.to_string()),
        };
        let target = if exists {
            path.canonicalize().map_err(|e| e.to_string())?
        } else {
            path.clone()
        };
        let same_source = d.path.iter().chain(d.source_path.iter()).any(|source| {
            let resolved = source.canonicalize().or_else(|_| {
                let parent = source
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                parent
                    .canonicalize()
                    .map(|parent| parent.join(source.file_name().unwrap_or_default()))
            });
            resolved.is_ok_and(|source| {
                if cfg!(windows) {
                    source
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&target.to_string_lossy())
                } else {
                    source == target
                }
            })
        });
        if same_source {
            return Err("Preserve the original source; choose a different PSD/PSB filename".into());
        }
        if exists && !replace {
            return Err("Confirm replacement of the existing export file".into());
        }
        Ok(())
    };
    check()?;
    let bytes = encode_with_preview(image, format, merged_preview, budget, cancel)?;
    canceled(cancel)?;
    let mut name = path.file_name().unwrap().to_owned();
    name.push(".psd-exchange.tmp");
    let temp = path.with_file_name(name);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        for chunk in bytes.chunks(65536) {
            canceled(cancel)?;
            file.write_all(chunk).map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        canceled(cancel)?;
        check()?;
        if replace {
            fs::rename(&temp, &path).map_err(|e| e.to_string())
        } else {
            fs::hard_link(&temp, &path).map_err(|e| e.to_string())
        }
    })();
    let _ = fs::remove_file(temp);
    result
}
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("Truncated PSD section / length overflow")?;
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn len(&mut self, f: Format) -> Result<usize, String> {
        let n = if f == Format::Psd {
            self.u32()? as u64
        } else {
            u64::from_be_bytes(self.take(8)?.try_into().unwrap())
        };
        usize::try_from(n).map_err(|_| "PSD length overflow".into())
    }
    fn section(&mut self, f: Format) -> Result<Cursor<'a>, String> {
        let n = self.len(f)?;
        Ok(Cursor::new(self.take(n)?))
    }
}
pub(super) fn unpack(data: &[u8], width: usize) -> Result<Vec<u8>, String> {
    let mut r = Cursor::new(data);
    let mut out = Vec::with_capacity(width);
    while r.left() > 0 {
        let n = r.byte()? as i8;
        match n {
            0..=127 => {
                let count = n as usize + 1;
                if out.len() + count > width {
                    return Err("PSD PackBits literal overflow".into());
                }
                out.extend(r.take(count)?);
            }
            -127..=-1 => {
                let count = (1i16 - n as i16) as usize;
                let value = r.byte()?;
                if out.len() + count > width {
                    return Err("PSD PackBits repeat overflow".into());
                }
                out.resize(out.len() + count, value);
            }
            _ => {}
        }
    }
    if out.len() != width {
        return Err("PSD PackBits short row".into());
    }
    Ok(out)
}
fn planes(
    r: &mut Cursor<'_>,
    w: usize,
    h: usize,
    count: usize,
    f: Format,
) -> Result<Vec<Vec<u8>>, String> {
    let method = r.u16()?;
    let mut out = Vec::with_capacity(count);
    match method {
        0 => {
            for _ in 0..count {
                out.push(
                    r.take(w.checked_mul(h).ok_or("PSD plane size overflow")?)?
                        .to_vec(),
                );
            }
        }
        1 => {
            let mut lengths = vec![];
            for _ in 0..h * count {
                lengths.push(if f == Format::Psd {
                    r.u16()? as usize
                } else {
                    r.u32()? as usize
                });
            }
            for c in 0..count {
                let mut plane = Vec::with_capacity(w * h);
                for y in 0..h {
                    plane.extend(unpack(r.take(lengths[c * h + y])?, w)?);
                }
                out.push(plane);
            }
        }
        _ => {
            return Err(format!(
                "Unsupported PSD compression {method}; verified import supports raw and PackBits RLE"
            ));
        }
    }
    Ok(out)
}
fn clean_name(name: String) -> String {
    let n = name
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>();
    if n.trim().is_empty() {
        "PSD layer".into()
    } else {
        n
    }
}
pub(super) fn decode(
    bytes: &[u8],
    budget: usize,
    cancel: &AtomicBool,
) -> Result<(Image, Vec<String>, bool), String> {
    if bytes.len() > budget / 2 {
        return Err("PSD file exceeds read memory limit".into());
    }
    let mut r = Cursor::new(bytes);
    let f = report::header(r.take(6)?)?;
    if r.take(6)? != [0; 6] {
        return Err("Invalid PSD reserved fields".into());
    }
    let channels = r.u16()? as usize;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let mode = r.u16()?;
    if depth != 8 || mode != 3 || !(3..=4).contains(&channels) {
        return Err(format!(
            "PSD color mode {mode}, depth {depth}, channels {channels} unsupported: verified exchange is RGB8 with optional transparency; CMYK/Lab/indexed/gray/16/32-bit require conversion before import"
        ));
    }
    if width == 0
        || height == 0
        || width > if f == Format::Psd { 30000 } else { 32768 }
        || height > if f == Format::Psd { 30000 } else { 32768 }
        || (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(32)
            > budget
    {
        return Err("PSD dimensions or decode work/memory limit".into());
    }
    let mut warnings = vec![];
    let color = r.section(Format::Psd)?;
    if color.left() != 0 {
        return Err("Unexpected RGB color-mode data".into());
    }
    let mut resources = r.section(Format::Psd)?;
    let mut dpi = 300.;
    let mut profile = None;
    let mut resource_ids = BTreeSet::new();
    let mut real_merged = true;
    while resources.left() > 0 {
        if resources.take(4)? != b"8BIM" {
            return Err("Invalid PSD resource signature".into());
        }
        let id = resources.u16()?;
        if !resource_ids.insert(id) {
            return Err("Duplicate PSD resource ID".into());
        }
        let n = resources.byte()? as usize;
        resources.take(n)?;
        if !(n + 1).is_multiple_of(2) {
            resources.take(1)?;
        }
        let n = resources.u32()? as usize;
        let data = resources.take(n)?;
        if !n.is_multiple_of(2) {
            resources.take(1)?;
        }
        if id == 1005 {
            let mut v = Cursor::new(data);
            let value = v.u32()? as f64 / 65536.;
            let unit = v.u16()?;
            if unit != 1 || !(1.0..=9600.0).contains(&value) {
                return Err("Unsupported PSD resolution units/value".into());
            }
            dpi = value;
        } else if id == 1057 {
            let mut v = Cursor::new(data);
            if v.u32()? != 1 {
                return Err("Unsupported PSD version-info resource".into());
            }
            real_merged = match v.byte()? {
                0 => false,
                1 => true,
                _ => return Err("Invalid PSD merged-data flag".into()),
            };
        } else if id == 1039 {
            profile = Some(crate::color_profile::RgbProfile::embedded(
                "PSD ICC".into(),
                data.to_vec(),
            )?);
        }
    }
    let mut section = r.section(f)?;
    let mut layers = vec![];
    let mut descriptors = vec![];
    let mut decoded = width as usize * height as usize * channels;
    let mut force_merged = false;
    let mut signed_count = 0i16;
    if section.left() > 0 {
        let mut info = section.section(f)?;
        if info.left() > 0 {
            signed_count = info.u16()? as i16;
            let count = signed_count.unsigned_abs() as usize;
            if count > 768 {
                return Err("PSD layer record count limit".into());
            }
            for _ in 0..count {
                canceled(cancel)?;
                let mut bounds = [0; 4];
                for n in &mut bounds {
                    *n = info.u32()? as i32;
                }
                let (w, h) = area(bounds)?;
                let count = info.u16()? as usize;
                if count > 8 {
                    return Err("PSD layer channel limit".into());
                }
                let mut ids = BTreeSet::new();
                let mut sizes = vec![];
                for _ in 0..count {
                    let id = info.u16()? as i16;
                    if !ids.insert(id) {
                        return Err("Duplicate PSD channel ID".into());
                    }
                    sizes.push((id, info.len(f)?));
                }
                if info.take(4)? != b"8BIM" {
                    return Err("Invalid PSD layer blend signature".into());
                }
                let mut blend: [u8; 4] = info.take(4)?.try_into().unwrap();
                let opacity = info.byte()?;
                let clipped = match info.byte()? {
                    0 => false,
                    1 => true,
                    _ => return Err("Invalid PSD clipping flag".into()),
                };
                let flags = info.byte()?;
                if info.byte()? != 0 {
                    return Err("Invalid PSD layer filler".into());
                }
                let mut extra = info.section(Format::Psd)?;
                let mut mask_data = extra.section(Format::Psd)?;
                let mask = if mask_data.left() > 0 {
                    let mut b = [0; 4];
                    for n in &mut b {
                        *n = mask_data.u32()? as i32;
                    }
                    area(b)?;
                    let default = mask_data.byte()?;
                    let mf = mask_data.byte()?;
                    if default != 0 && default != 255 {
                        return Err("Invalid PSD mask default".into());
                    }
                    if mf & !7 != 0 || mask_data.left() != 2 {
                        force_merged = true;
                        warnings.push("Mask density/feather/real-mask parameters require the merged substitute".into());
                    }
                    if mf & 1 != 0 {
                        b[0] = b[0]
                            .checked_add(bounds[0])
                            .ok_or("Relative mask overflow")?;
                        b[2] = b[2]
                            .checked_add(bounds[0])
                            .ok_or("Relative mask overflow")?;
                        b[1] = b[1]
                            .checked_add(bounds[1])
                            .ok_or("Relative mask overflow")?;
                        b[3] = b[3]
                            .checked_add(bounds[1])
                            .ok_or("Relative mask overflow")?;
                    }
                    Some(Mask {
                        bounds: b,
                        default,
                        flags: mf & 6,
                        pixels: vec![],
                    })
                } else {
                    None
                };
                let ranges = extra.section(Format::Psd)?;
                if !ranges.bytes.len().is_multiple_of(8)
                    || ranges
                        .bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|range| *range != [0, 0, 255, 255])
                {
                    force_merged = true;
                    warnings.push("Layer blending ranges require the merged substitute".into());
                }
                let n = extra.byte()? as usize;
                let mut name = String::from_utf8_lossy(extra.take(n)?).into_owned();
                let pad = (4 - (n + 1) % 4) % 4;
                extra.take(pad)?;
                let mut group = 0;
                let mut tags = BTreeSet::new();
                while extra.left() > 0 {
                    if extra.left() < 12 {
                        if extra.take(extra.left())?.iter().any(|n| *n != 0) {
                            return Err("PSD tagged-block padding".into());
                        }
                        break;
                    }
                    let signature = extra.take(4)?;
                    if signature != b"8BIM" && signature != b"8B64" {
                        return Err("Invalid PSD tagged-block signature".into());
                    }
                    let key: [u8; 4] = extra.take(4)?.try_into().unwrap();
                    if !tags.insert(key) {
                        return Err("Duplicate PSD tagged block".into());
                    }
                    let wide = f == Format::Psb
                        && [
                            *b"LMsk", *b"Lr16", *b"Lr32", *b"Layr", *b"Mt16", *b"Mt32", *b"Mtrn",
                            *b"Alph", *b"FMsk", *b"lnk2", *b"FEid", *b"FXid", *b"PxSD",
                        ]
                        .contains(&key);
                    let len = extra.len(if wide { Format::Psb } else { Format::Psd })?;
                    let data = extra.take(len)?;
                    if !len.is_multiple_of(2) {
                        extra.take(1)?;
                    }
                    match &key{b"luni"=>{let mut text=Cursor::new(data);let n=text.u32()? as usize;if n>4096{return Err("PSD layer name limit".into());}let mut chars=vec![];for _ in 0..n{chars.push(text.u16()?);}name=String::from_utf16(&chars).map_err(|e|e.to_string())?;},b"lsct"|b"lsdk"=>{let mut v=Cursor::new(data);group=v.u32()?;if group>3{return Err("Invalid PSD group divider".into());}
if v.left()>=8{if v.take(4)?!=b"8BIM"{return Err("Invalid PSD group blend".into());}blend=v.take(4)?.try_into().unwrap();}},b"lyid"|b"lspf"|b"lclr"|b"lnsr"|b"shmd"=>{},b"TySh"|b"tySh"=>warnings.push(format!("{name}: text/font/ruby/layout editing replaced by the cached raster if its composite validates")),b"SoLd"|b"SoLE"|b"PlLd"|b"plLd"=>warnings.push(format!("{name}: smart object/source editing replaced by cached pixels if its composite validates")),_=>{force_merged=true;warnings.push(format!("{name}: unsupported layer block {} requires the merged substitute",String::from_utf8_lossy(&key)));}}
                }
                if flags & 16 != 0 {
                    force_merged = true;
                    warnings.push(format!(
                        "{name}: adjustment/non-pixel source requires the merged substitute"
                    ));
                }
                if blend != *b"pass" && blend_mode(blend).is_err() {
                    force_merged = true;
                    warnings.push(format!(
                        "{name}: unsupported blend {}",
                        String::from_utf8_lossy(&blend)
                    ));
                }
                if sizes.iter().any(|(id, _)| ![-2, -1, 0, 1, 2].contains(id)) {
                    force_merged = true;
                    warnings.push(format!(
                        "{name}: extra/vector/spot channel requires merged substitution"
                    ));
                }
                decoded = decoded
                    .saturating_add(w.saturating_mul(h).saturating_mul(4))
                    .saturating_add(
                        mask.as_ref()
                            .map_or(0, |m| area(m.bounds).map_or(usize::MAX, |(w, h)| w * h)),
                    );
                if decoded.saturating_mul(4) > budget {
                    return Err("PSD decoded layer/mask memory limit".into());
                }
                layers.push(Layer {
                    name: clean_name(name),
                    bounds,
                    pixels: if group == 0 {
                        vec![[0, 0, 0, 255]; w * h]
                    } else {
                        vec![]
                    },
                    mask,
                    blend,
                    opacity,
                    hidden: flags & 2 != 0,
                    clipped,
                    alpha_lock: flags & 1 != 0,
                    section: group,
                });
                descriptors.push(sizes);
            }
            for (layer, sizes) in layers.iter_mut().zip(descriptors) {
                let (w, h) = area(layer.bounds)?;
                let mut got = BTreeSet::new();
                for (id, size) in sizes {
                    canceled(cancel)?;
                    let mut data = Cursor::new(info.take(size)?);
                    let (mw, mh) = if id == -2 {
                        layer.mask.as_ref().map_or(Ok((w, h)), |m| area(m.bounds))?
                    } else {
                        (w, h)
                    };
                    let plane = planes(&mut data, mw, mh, 1, f)?.remove(0);
                    if data.left() > 1 || data.left() == 1 && data.byte()? != 0 {
                        return Err("PSD channel trailing data".into());
                    }
                    if id == -2 {
                        if let Some(mask) = &mut layer.mask {
                            mask.pixels = plane;
                        } else {
                            return Err("PSD mask channel missing its rectangle".into());
                        }
                    } else if layer.section == 0 && [-1, 0, 1, 2].contains(&id) {
                        let c = if id == -1 { 3 } else { id as usize };
                        for (p, v) in layer.pixels.iter_mut().zip(plane) {
                            p[c] = v;
                        }
                        got.insert(id);
                    }
                }
                if layer.section == 0
                    && (!got.contains(&0) || !got.contains(&1) || !got.contains(&2))
                {
                    force_merged = true;
                    warnings.push(format!(
                        "{}: missing usable raster channels; use merged substitution",
                        layer.name
                    ));
                }
                if layer.mask.as_ref().is_some_and(|m| {
                    m.pixels.is_empty() && area(m.bounds).is_ok_and(|(w, h)| w * h > 0)
                }) {
                    return Err("Missing PSD mask pixels".into());
                }
            }
            if info.left() > 1 || info.left() == 1 && info.byte()? != 0 {
                return Err("PSD layer-info trailing bytes".into());
            }
        }
        if section.left() > 0 {
            let global = section.section(Format::Psd)?;
            if global.left() > 0 {
                force_merged = true;
                warnings.push("Global layer-mask information requires merged substitution".into());
            }
            if section.left() > 0 {
                force_merged = true;
                warnings.push("Additional global layer data requires merged substitution".into());
            }
        }
    }
    let plane = planes(&mut r, width as usize, height as usize, channels, f)?;
    if r.left() != 0 {
        return Err("PSD merged image trailing bytes".into());
    }
    let count = width as usize * height as usize;
    let mut merged = vec![[0, 0, 0, 255]; count];
    for c in 0..channels {
        for (p, v) in merged.iter_mut().zip(&plane[c]) {
            if c < 3 || signed_count < 0 {
                p[c] = *v;
            }
        }
    }
    if channels == 4 && signed_count >= 0 {
        warnings.push(
            "Non-transparency auxiliary alpha channel omitted; it is not document transparency"
                .into(),
        );
    }
    if channels == 4 && signed_count < 0 {
        for p in &mut merged {
            let alpha = p[3] as i64;
            for c in &mut p[..3] {
                *c = if alpha == 0 {
                    0
                } else {
                    (((*c as i64 * 255 - 255 * (255 - alpha)) * 2 + alpha) / (2 * alpha))
                        .clamp(0, 255) as u8
                };
            }
        }
    }
    layers.reverse();
    if let Some(profile) = profile {
        let own = matches!(&profile,crate::color_profile::RgbProfile::Embedded{bytes,..}if *bytes==srgb_icc()?);
        if !own {
            let t = crate::color_profile::Transform::new(
                &profile,
                &crate::color_profile::RgbProfile::Srgb,
            )?;
            for p in &mut merged {
                *p = t.map(crate::color_precision::Sample::from8(*p))?.rgba8();
            }
            force_merged = true;
            warnings.push("Embedded RGB ICC converted on the merged image; editable cross-profile layer blending is not verified".into());
        }
    } else {
        warnings.push("PSD has no ICC profile; assuming sRGB".into());
    }
    let mut image = Image {
        width,
        height,
        dpi,
        layers,
        merged,
    };
    if !real_merged {
        if force_merged || image.layers.is_empty() {
            return Err("This PSD has no real merged preview and its layer composition is unverified; export a compatible merged preview in the source app".into());
        }
        let document = image.document(budget)?;
        image.merged = render(&document, budget, cancel)?;
        warnings.push("No embedded merged preview (Version Info hasRealMergedData=false); appearance rebuilt from the verified raster layers".into());
    }
    Ok((image, warnings, force_merged))
}
pub fn open_report(path: &Path, budget: usize) -> Result<(Document, Vec<String>), String> {
    open_detailed(path, budget).map(|(d, w, _)| (d, w))
}
pub fn open_detailed(
    path: &Path,
    budget: usize,
) -> Result<(Document, Vec<String>, report::Report), String> {
    crate::exchange_guidance::reject_private(path)?;
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > budget as u64 / 2 {
        return Err("Choose a regular local PSD/PSB within the read budget".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&path) {
        return Err("PSD/PSB import is local only".into());
    }
    let mut bytes = vec![];
    fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(budget as u64 / 2 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let (image, mut warnings, force) = decode(&bytes, budget, &cancel)?;
    let candidate = image.document(budget);
    let preserved = if !force {
        match candidate.as_ref() {
            Ok(d) => difference(&image.merged, &render(d, budget, &cancel)?) <= 1,
            Err(_) => false,
        }
    } else {
        false
    };
    let mut report = report::Report::image(&image, report::header(&bytes)?, !preserved, &warnings);
    let mut d = if preserved {
        candidate.unwrap()
    } else {
        let detail = if force {
            "unsupported editing/composition data".into()
        } else {
            candidate
                .err()
                .unwrap_or("layer composite differs from the embedded preview".into())
        };
        warnings.push(format!("Whole merged substitute used: {detail}. Separate layers/groups/masks/text/smart-object/effect editing was not imported; preserve the source PSD/PSB."));
        let mut d = Document::create(
            CanvasSpec {
                width: image.width,
                height: image.height,
                dpi: image.dpi,
                background: Background::Transparent,
            },
            budget,
        )
        .map_err(|e| e.to_string())?;
        d.layers[0].name = "PSD merged substitute".into();
        d.layers[0].pixels = image.merged;
        d
    };
    d.title = path.file_name().unwrap().to_string_lossy().into_owned();
    d.source_path = Some(path);
    report.notes = warnings.clone();
    Ok((d, warnings, report))
}
