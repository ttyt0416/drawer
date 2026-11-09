//! Import provenance is descriptive data, never permission to fetch or execute.
use serde::{Deserialize, Serialize};

pub const EXTENSIONS: &[&str] = &["obj", "gltf", "glb", "vrm", "fbx", "stl"];
pub const SUPPORT: &[(&str, &str, &str)] = &[
    (
        "OBJ + MTL",
        "ASCII / UTF-8 polygon OBJ",
        "Mesh/UV/normals, diffuse PNG/JPEG and opacity. No standard rig/morph or terms field; comments retained as possible notices. Advanced MTL statements reported.",
    ),
    (
        "glTF / GLB",
        "glTF 2.0 / GLB 2",
        "Triangles/strips/fans, base color/alpha maps; default LBS pose and joint metadata; position morphs become editable shape keys. No animation/PBR shading; unsupported required extensions reject.",
    ),
    (
        "VRM",
        "GLB 2 with VRM 0.x / VRMC_vrm 1.0",
        "glTF geometry/skin/morph conversion; raw avatar metadata and usage terms retained. Humanoid mapping/expressions retained as source metadata, not retargeted. MToon, spring bones and constraints are reported, not simulated.",
    ),
    (
        "FBX",
        "ASCII / binary 7.1–7.7 (7100–7700)",
        "Polygon meshes, UV/normals, Lambert/Phong diffuse/opacity PNG/JPEG, linear skin frozen with joints/weights; single-target position blend shapes become editable keys. Default pose only; advanced shaders/animation/cache not converted.",
    ),
    (
        "STL",
        "ASCII / little-endian binary",
        "Triangle geometry; face normals regenerated. No standard UV, texture, alpha, rig, morph or usage terms; binary header / ASCII solid label retained. Nonstandard facet colors reported.",
    ),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    pub source: String,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub format: String,
    pub version: String,
    pub file: String,
    pub notices: Vec<Notice>,
    pub usage_present: bool,
}
fn text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains('\0')
}
impl Source {
    pub fn valid(&self) -> bool {
        text(&self.format, 32)
            && text(&self.version, 128)
            && text(&self.file, 32768)
            && self.notices.len() <= 64
            && self
                .notices
                .iter()
                .all(|n| text(&n.source, 1024) && text(&n.text, 65536))
            && self.bytes() <= 256 * 1024
    }
    pub fn bytes(&self) -> usize {
        self.format.len()
            + self.version.len()
            + self.file.len()
            + self
                .notices
                .iter()
                .map(|n| n.source.len() + n.text.len() + 48)
                .sum::<usize>()
    }
    pub fn push(
        &mut self,
        source: impl Into<String>,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        if value.is_null() {
            return Ok(());
        }
        fn readable(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::String(s) => !s.contains('\0'),
                serde_json::Value::Array(a) => a.iter().all(readable),
                serde_json::Value::Object(o) => {
                    o.iter().all(|(k, v)| !k.contains('\0') && readable(v))
                }
                _ => true,
            }
        }
        if !readable(value) {
            return Err("Unreadable NUL in model usage text".into());
        }
        fn usage(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::Object(o) => o.iter().any(|(k, v)| {
                    ["license", "copyright", "usage", "terms"]
                        .iter()
                        .any(|word| k.to_ascii_lowercase().contains(word))
                        || usage(v)
                }),
                serde_json::Value::Array(a) => a.iter().any(usage),
                _ => false,
            }
        }
        self.usage_present |= usage(value);
        self.notices.push(Notice {
            source: source.into(),
            text: value.as_str().map_or_else(
                || serde_json::to_string_pretty(value).map_err(|e| e.to_string()),
                |s| Ok(s.into()),
            )?,
        });
        if !self.valid() {
            return Err(
                "Model usage/source metadata exceeds limits or contains unreadable NUL text".into(),
            );
        }
        Ok(())
    }
}

pub(crate) fn attach(a: &mut crate::model_mesh::Asset, source: Source) -> Result<(), String> {
    if !source.valid() {
        return Err("Invalid model source / usage information".into());
    }
    a.warning(format!("{} {}: source / usage notes {}. Metadata describes the source; it does not grant permission. Review the stored original text before reuse.",source.format,source.version,if source.notices.is_empty(){"not provided"}else{"present (see Local models / source terms)"}));
    for m in &mut a.models {
        m.surface.imported.get_or_insert_default().source = Some(source.clone());
    }
    Ok(())
}

/// OBJ has no standard license syntax. Preserve comments without interpreting them.
pub(crate) fn obj(path: &std::path::Path, a: &mut crate::model_mesh::Asset) -> Result<(), String> {
    let mut s = Source {
        format: "OBJ".into(),
        version: "polygon text + MTL".into(),
        file: path.display().to_string(),
        notices: vec![],
        usage_present: false,
    };
    let mut paths = vec![path.to_owned()];
    for r in a
        .models
        .iter()
        .flat_map(|m| &m.surface.references)
        .filter(|r| !r.texture && r.present)
    {
        let p =
            crate::model_import::resolve(path.parent().ok_or("Missing model folder")?, &r.name)?;
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    for p in paths {
        let bytes = crate::book_io::read_local(&p, 8 * 1024 * 1024)?;
        let raw =
            std::str::from_utf8(&bytes).map_err(|_| "Model comments / usage text must be UTF-8")?;
        let comments = raw
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix('#'))
            .collect::<Vec<_>>()
            .join("\n");
        if !comments.trim().is_empty() {
            if comments.len() > 65536 || s.notices.len() >= 64 {
                return Err("Model comments exceed metadata limits".into());
            }
            let words = comments.to_ascii_lowercase();
            s.usage_present |= ["license", "copyright", "usage", "terms", "사용조건"]
                .iter()
                .any(|w| words.contains(w));
            s.notices.push(Notice {
                source: format!(
                    "{} # comments (unclassified)",
                    p.file_name().unwrap_or_default().to_string_lossy()
                ),
                text: comments,
            });
            if !s.valid() {
                return Err("Model source comments exceed metadata limits".into());
            }
        }
    }
    attach(a, s)
}
