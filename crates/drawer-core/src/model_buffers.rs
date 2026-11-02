//! Validated glTF resources and little-endian accessors (no sparse/codec extensions).
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value as J;
use std::{path::Path, sync::atomic::AtomicBool};
pub(crate) fn number(v: &J) -> Result<usize, String> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "Expected a nonnegative model index/length".into())
}
pub(crate) fn field(v: &J, k: &str) -> Result<usize, String> {
    number(
        v.get(k)
            .ok_or_else(|| format!("Missing model property: {k}"))?,
    )
}
pub(crate) fn optional(v: &J, k: &str, default: usize) -> Result<usize, String> {
    v.get(k).map_or(Ok(default), number)
}
pub(crate) fn array<'a>(v: &'a J, k: &str) -> Result<&'a [J], String> {
    match v.get(k) {
        None => Ok(&[]),
        Some(v) => v
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("Expected model array: {k}")),
    }
}
pub(crate) fn at<'a>(v: &'a J, k: &str, i: usize) -> Result<&'a J, String> {
    array(v, k)?
        .get(i)
        .ok_or_else(|| format!("Model index outside {k}"))
}
pub(crate) fn label(v: &J, fallback: &str) -> Result<String, String> {
    let n = v
        .get("name")
        .map_or(Ok(fallback), |v| v.as_str().ok_or("Expected model name"))?;
    if n.trim().is_empty() || n.len() > 1024 || n.chars().any(char::is_control) {
        Err("Invalid model name".into())
    } else {
        Ok(n.into())
    }
}
pub(crate) struct Resources<'a> {
    pub json: &'a J,
    pub buffers: Vec<Vec<u8>>,
    pub root: &'a Path,
    pub budget: usize,
    pub used: usize,
}
impl<'a> Resources<'a> {
    pub fn new(
        json: &'a J,
        bin: Option<&[u8]>,
        root: &'a Path,
        budget: usize,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let mut r = Self {
            json,
            buffers: vec![],
            root,
            budget,
            used: 0,
        };
        for (i, b) in array(json, "buffers")?.iter().enumerate() {
            crate::book_io::check_cancel(cancel)?;
            let len = field(b, "byteLength")?;
            if len == 0 || len > 16 * 1024 * 1024 {
                return Err("Buffer byte length limit16MiB".into());
            }
            let bytes = if let Some(uri) = b.get("uri") {
                r.uri(uri.as_str().ok_or("Buffer URI must be text")?, false)?
            } else if i == 0 {
                let bytes = bin.ok_or("A URI-less buffer requires GLB BIN data")?;
                if bytes.len() < len || bytes.len() > len + 3 {
                    return Err("GLB BIN length/padding differs from declared buffer".into());
                }
                r.reserve(bytes.len())?;
                bytes[..len].to_vec()
            } else {
                return Err("Only first GLB buffer may omit its URI".into());
            };
            if bytes.len() < len {
                return Err("Buffer shorter than declared byteLength".into());
            }
            r.buffers.push(bytes[..len].to_vec());
        }
        for view in array(json, "bufferViews")? {
            r.view(view)?;
        }
        Ok(r)
    }
    fn reserve(&mut self, n: usize) -> Result<(), String> {
        self.used = self.used.saturating_add(n);
        if self.used > 32 * 1024 * 1024 || self.used.saturating_mul(8) > self.budget {
            return Err("Model resource reservation exceeds memory budget".into());
        }
        Ok(())
    }
    pub fn uri(&mut self, uri: &str, image: bool) -> Result<Vec<u8>, String> {
        let bytes = if uri.starts_with("data:") {
            let (head, data) = uri.split_once(',').ok_or("Invalid embedded data URI")?;
            let valid = if image {
                matches!(head, "data:image/png;base64" | "data:image/jpeg;base64")
            } else {
                matches!(
                    head,
                    "data:application/octet-stream;base64" | "data:application/gltf-buffer;base64"
                )
            };
            if !valid || data.len() > 24 * 1024 * 1024 || data.len().saturating_mul(8) > self.budget
            {
                return Err("Unsupported or oversized data URI".into());
            }
            STANDARD.decode(data).map_err(|e| e.to_string())?
        } else {
            let name = crate::model_import::uri_name(uri)?;
            let p = crate::model_import::resolve(self.root, &name)?;
            crate::book_io::read_local(&p, (16 * 1024 * 1024).min(self.budget / 8))?
        };
        self.reserve(bytes.len())?;
        Ok(bytes)
    }
    fn view(&self, v: &J) -> Result<&[u8], String> {
        let buffer = self
            .buffers
            .get(field(v, "buffer")?)
            .ok_or("Unknown buffer")?;
        let start = optional(v, "byteOffset", 0)?;
        let length = field(v, "byteLength")?;
        if length == 0 {
            return Err("Empty buffer view".into());
        }
        buffer
            .get(start..start.checked_add(length).ok_or("Buffer range overflow")?)
            .ok_or_else(|| "Buffer view exceeds its buffer".into())
    }
    pub fn image(&mut self, i: usize) -> Result<crate::model_import::Image, String> {
        let image = at(self.json, "images", i)?;
        let bytes = match (image.get("uri"), image.get("bufferView")) {
            (Some(uri), None) => self.uri(uri.as_str().ok_or("Image URI must be text")?, true)?,
            (None, Some(i)) => {
                if !matches!(
                    image.get("mimeType").and_then(J::as_str),
                    Some("image/png" | "image/jpeg")
                ) {
                    return Err("Embedded image needs PNG/JPEG MIME type".into());
                }
                self.view(at(self.json, "bufferViews", number(i)?)?)?
                    .to_vec()
            }
            _ => return Err("Image requires exactly one URI or bufferView".into()),
        };
        if let Some(mime) = image.get("mimeType") {
            let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
            let actual = match format {
                image::ImageFormat::Png => "image/png",
                image::ImageFormat::Jpeg => "image/jpeg",
                _ => "unsupported",
            };
            if mime.as_str() != Some(actual) {
                return Err("Image MIME type differs from encoded image".into());
            }
        }
        let result = crate::model_import::Image::from_bytes(&bytes)?;
        self.reserve(result.bytes())?;
        Ok(result)
    }
    pub fn accessor(
        &self,
        i: usize,
        kind: &str,
        components: &[usize],
        limit: usize,
    ) -> Result<Vec<Vec<f64>>, String> {
        let a = at(self.json, "accessors", i)?;
        if a.get("sparse").is_some() {
            return Err(format!(
                "Sparse accessor {i} is unsupported; densify it before import"
            ));
        }
        if a.get("type").and_then(J::as_str) != Some(kind) {
            return Err(format!("Accessor {i} must have {kind} type"));
        }
        let c = field(a, "componentType")?;
        if !components.contains(&c) {
            return Err(format!("Unsupported component type {c} in accessor {i}"));
        }
        let size = match c {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err("Unsupported accessor component".into()),
        };
        let columns = match kind {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            _ => return Err("Unsupported accessor shape".into()),
        };
        let count = field(a, "count")?;
        if count == 0
            || count > limit
            || count.saturating_mul(columns * 8).saturating_mul(8) > self.budget
        {
            return Err("Accessor count/memory limit".into());
        }
        let normalized = match a.get("normalized") {
            None => false,
            Some(v) => v.as_bool().ok_or("normalized must be boolean")?,
        };
        if normalized && matches!(c, 5125 | 5126) {
            return Err("Invalid normalized accessor component".into());
        }
        let v = at(self.json, "bufferViews", field(a, "bufferView")?)?;
        let bytes = self.view(v)?;
        let start = optional(a, "byteOffset", 0)?;
        let stride = optional(v, "byteStride", size * columns)?;
        if stride < size * columns
            || (v.get("byteStride").is_some() && (!(4..=252).contains(&stride) || stride % 4 != 0))
            || start % size != 0
            || (optional(v, "byteOffset", 0)? + start) % size != 0
        {
            return Err("Invalid accessor stride/alignment".into());
        }
        let end = start
            .checked_add(
                (count - 1)
                    .checked_mul(stride)
                    .ok_or("Accessor range overflow")?,
            )
            .and_then(|n| n.checked_add(size * columns))
            .ok_or("Accessor range overflow")?;
        if end > bytes.len() {
            return Err("Accessor exceeds buffer view".into());
        }
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let mut row = Vec::with_capacity(columns);
            for j in 0..columns {
                let p = start + i * stride + j * size;
                let b = &bytes[p..p + size];
                let n = match c {
                    5120 => {
                        let n = b[0] as i8 as f64;
                        if normalized { (n / 127.).max(-1.) } else { n }
                    }
                    5121 => {
                        let n = b[0] as f64;
                        if normalized { n / 255. } else { n }
                    }
                    5122 => {
                        let n = i16::from_le_bytes(b.try_into().unwrap()) as f64;
                        if normalized { (n / 32767.).max(-1.) } else { n }
                    }
                    5123 => {
                        let n = u16::from_le_bytes(b.try_into().unwrap()) as f64;
                        if normalized { n / 65535. } else { n }
                    }
                    5125 => u32::from_le_bytes(b.try_into().unwrap()) as f64,
                    5126 => f32::from_le_bytes(b.try_into().unwrap()) as f64,
                    _ => unreachable!(),
                };
                if !n.is_finite() || n.abs() > 1e9 {
                    return Err("Non-finite/excessive accessor value".into());
                }
                row.push(n);
            }
            out.push(row);
        }
        Ok(out)
    }
}
