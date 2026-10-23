//! CPU-only ONNX adapters. No network provider, downloads, custom ops or external tensor files.
use anyhow::{ensure, format_err};
use drawer_core::{
    local_inference::{self as core, Output, Task},
    references::LocalImage,
};
use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
use tract_onnx::{pb, prelude::*};
mod process;
pub use process::execute;
pub mod comparison;
pub mod fixtures;
#[cfg(test)]
mod verify;

pub const GUIDE: &str = "Install an independently licensed local model bundle: manifest.json, weights.onnx and LICENSE.txt in one folder. Choose its manifest below. No models are downloaded or bundled. ONNX export contracts and a fixed comparison set are in Help / Local models. A hash verifies bytes, not model quality or redistribution permission.";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    pub version: String,
    pub license: String,
    pub source: String,
    pub weights_sha256: String,
    pub license_sha256: String,
    pub task: Task,
    pub width: usize,
    pub height: usize,
    pub scale: usize,
    pub mean: [f32; 3],
    pub std: [f32; 3],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub manifest: String,
    pub task: Task,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub report: String,
    pub points: Vec<[f32; 3]>,
    pub error: Option<String>,
}
pub fn read_bytes(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !f.metadata().map_err(|e| e.to_string())?.is_file()
        || f.metadata().map_err(|e| e.to_string())?.len() > max as u64
    {
        return Err("Local model file exceeds memory limit".into());
    }
    let mut bytes = vec![];
    f.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err("Local file grew beyond memory limit".into());
    }
    Ok(bytes)
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn inspect(path: &Path, task: Task) -> Result<(Manifest, Vec<u8>), String> {
    if !path.is_file() {
        return Err(GUIDE.into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("Missing model folder")?;
    let m: Manifest = serde_json::from_slice(&read_bytes(&path, 16384)?)
        .map_err(|e| format!("Invalid model manifest: {e}"))?;
    let text = |s: &str| !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control);
    if m.format != 1
        || m.task != task
        || ![&m.name, &m.version, &m.license, &m.source]
            .iter()
            .all(|s| text(s))
        || m.width == 0
        || m.height == 0
        || m.width > 512
        || m.height > 512
        || !m.mean.iter().all(|v| v.is_finite() && v.abs() <= 10.)
        || !m
            .std
            .iter()
            .all(|v| v.is_finite() && (0.0001..=10.).contains(v))
        || !(1..=4).contains(&m.scale)
        || (task != Task::Upscale && m.scale != 1)
    {
        return Err(
            "Invalid model version/task/normalization/dimensions; see Local models help".into(),
        );
    }
    let file = |name: &str, max: usize, expected: &str| -> Result<Vec<u8>, String> {
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err("A lowercase SHA-256 is required for weights and license".into());
        }
        let p = parent
            .join(name)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if p.parent() != Some(parent) {
            return Err("Model files must remain inside their bundle".into());
        }
        let bytes = read_bytes(&p, max)?;
        if hash(&bytes) != expected {
            return Err(format!("SHA-256 mismatch: {name}; model was not run"));
        }
        Ok(bytes)
    };
    let license = file("LICENSE.txt", 65536, &m.license_sha256)?;
    if std::str::from_utf8(&license).is_err() || license.iter().all(u8::is_ascii_whitespace) {
        return Err("A nonempty UTF-8 model license is required".into());
    }
    let bytes = file("weights.onnx", 64 * 1024 * 1024, &m.weights_sha256)?;
    Ok((m, bytes))
}
fn tensor(t: &pb::TensorProto) -> Result<(), String> {
    if !t.external_data.is_empty()
        || t.data_location.unwrap_or(0) != 0
        || t.dims.len() > 8
        || t.dims
            .iter()
            .try_fold(1u64, |n, d| n.checked_mul(u64::try_from(*d).ok()?))
            .is_none_or(|n| n > 16 * 1024 * 1024)
    {
        return Err("External or oversized tensor data is unsupported".into());
    }
    Ok(())
}
fn preflight(p: &pb::ModelProto) -> Result<(), String> {
    let g = p.graph.as_ref().ok_or("ONNX graph missing")?;
    if !p.functions.is_empty()
        || !p.training_info.is_empty()
        || !g.sparse_initializer.is_empty()
        || g.node.len() > 4096
        || g.initializer.len() > 4096
        || p.opset_import
            .iter()
            .any(|o| !o.domain.is_empty() && o.domain != "ai.onnx")
    {
        return Err("Only a bounded standard ONNX graph is supported".into());
    }
    for t in &g.initializer {
        tensor(t)?;
    }
    for n in &g.node {
        if !n.domain.is_empty() && n.domain != "ai.onnx"
            || !matches!(
                n.op_type.as_str(),
                "Conv"
                    | "Gemm"
                    | "MatMul"
                    | "Relu"
                    | "LeakyRelu"
                    | "PRelu"
                    | "Resize"
                    | "Reshape"
                    | "Flatten"
                    | "GlobalAveragePool"
                    | "AveragePool"
                    | "MaxPool"
                    | "Add"
                    | "Mul"
                    | "Sub"
                    | "Div"
                    | "BatchNormalization"
                    | "InstanceNormalization"
                    | "Sigmoid"
                    | "Tanh"
                    | "Concat"
                    | "Transpose"
                    | "Clip"
                    | "Identity"
                    | "ReduceMean"
                    | "Softmax"
                    | "Constant"
                    | "Squeeze"
                    | "Unsqueeze"
                    | "DepthToSpace"
            )
        {
            return Err(format!(
                "Unsupported local model operator: {} / {}",
                n.domain, n.op_type
            ));
        }
        for a in &n.attribute {
            if a.g.is_some()
                || !a.graphs.is_empty()
                || a.sparse_tensor.is_some()
                || !a.sparse_tensors.is_empty()
            {
                return Err("Subgraphs and sparse tensors are unsupported".into());
            }
            if let Some(t) = &a.t {
                tensor(t)?;
            }
            for t in &a.tensors {
                tensor(t)?;
            }
        }
    }
    Ok(())
}
/// Bilinear, alpha-aware sRGB sampling; hidden RGB never enters inference.
fn sample(i: &LocalImage, x: f64, y: f64) -> [f32; 4] {
    let x = x.clamp(0., i.width as f64 - 1.);
    let y = y.clamp(0., i.height as f64 - 1.);
    let a = x.floor() as u32;
    let b = y.floor() as u32;
    let fx = (x - a as f64) as f32;
    let fy = (y - b as f64) as f32;
    let mut out = [0.; 4];
    for (px, py, w) in [
        (a, b, (1. - fx) * (1. - fy)),
        ((a + 1).min(i.width - 1), b, fx * (1. - fy)),
        (a, (b + 1).min(i.height - 1), (1. - fx) * fy),
        ((a + 1).min(i.width - 1), (b + 1).min(i.height - 1), fx * fy),
    ] {
        let p = i.pixels[(py * i.width + px) as usize];
        let alpha = p[3] as f32 / 255.;
        out[3] += w * alpha;
        for c in 0..3 {
            out[c] += w * alpha * p[c] as f32 / 255.;
        }
    }
    if out[3] > 0. {
        for c in 0..3 {
            out[c] /= out[3];
        }
    }
    out
}
pub fn run(
    path: &Path,
    task: Task,
    input: &LocalImage,
    hints: Option<&LocalImage>,
) -> Result<Output, String> {
    if !core::valid_image(input, core::MAX_INPUT) || input.pixels.iter().all(|p| p[3] == 0) {
        return Err("Choose a nontransparent image within the input memory limit".into());
    }
    let (m, bytes) = inspect(path, task)?;
    if task == Task::HintColor
        && hints.is_none_or(|h| {
            !core::valid_image(h, core::MAX_INPUT)
                || h.width != input.width
                || h.height != input.height
                || h.pixels.iter().all(|p| p[3] == 0)
        })
    {
        return Err("Hint colorization requires a same-size local RGBA hint image with at least one colored hint".into());
    }
    let channels = if task == Task::HintColor { 7 } else { 3 };
    let area = m.width * m.height;
    let mut values = vec![0f32; channels * area];
    for y in 0..m.height {
        for x in 0..m.width {
            let sx = (x as f64 + 0.5) * input.width as f64 / m.width as f64 - 0.5;
            let sy = (y as f64 + 0.5) * input.height as f64 / m.height as f64 - 0.5;
            let p = sample(input, sx, sy);
            let k = y * m.width + x;
            for c in 0..3 {
                values[c * area + k] = (p[c] * p[3] + 1. - p[3] - m.mean[c]) / m.std[c];
            }
            if task == Task::HintColor {
                let h = sample(hints.unwrap(), sx, sy);
                for c in 0..4 {
                    values[(c + 3) * area + k] = h[c];
                }
            }
        }
    }
    let proto =
        pb::ModelProto::decode(bytes.as_slice()).map_err(|e| format!("Invalid ONNX: {e}"))?;
    preflight(&proto)?;
    let evaluate = || -> TractResult<TVec<TValue>> {
        let mut model = tract_onnx::onnx().model_for_proto_model(&proto)?;
        ensure!(
            model.input_outlets()?.len() == 1 && model.output_outlets()?.len() == 1,
            "The adapter requires one input and one output"
        );
        model.set_input_fact(0, f32::fact([1, channels, m.height, m.width]).into())?;
        let model = model.into_typed()?;
        let mut total = 0usize;
        for node in model.nodes() {
            for o in &node.outputs {
                let shape = o.fact.shape.as_concrete().ok_or_else(|| {
                    format_err!("Dynamic intermediate dimensions are unsupported")
                })?;
                let n = shape
                    .iter()
                    .try_fold(1usize, |a, b| a.checked_mul(*b))
                    .ok_or_else(|| format_err!("Tensor size overflow"))?;
                ensure!(
                    n <= 8 * 1024 * 1024,
                    "Model intermediate exceeds memory limit"
                );
                total = total.saturating_add(n.saturating_mul(o.fact.datum_type.size_of()));
            }
        }
        ensure!(
            total <= 128 * 1024 * 1024,
            "Model graph exceeds the inference memory estimate"
        );
        let expected = if task.joints() > 0 {
            vec![1, task.joints(), 3]
        } else {
            vec![1, 3, m.height * m.scale, m.width * m.scale]
        };
        let out = model.output_fact(0)?;
        ensure!(
            out.datum_type == f32::datum_type()
                && out.shape.as_concrete() == Some(expected.as_slice()),
            "Model output does not match the selected adapter"
        );
        let tensor = Tensor::from_shape(&[1, channels, m.height, m.width], &values)?;
        model.into_runnable()?.run(tvec!(tensor.into_tvalue()))
    };
    let tensors = evaluate().map_err(|e| format!("Local ONNX inference failed: {e:#}"))?;
    let floats = tensors[0].as_slice::<f32>().map_err(|e| e.to_string())?;
    if floats.iter().any(|v| !v.is_finite()) {
        return Err("Model produced nonfinite output".into());
    }
    let report = format!(
        "{} {} | declared license: {} | source: {} | SHA-256 {} | CPU / no transmission. License text and weight hashes matched; quality and redistribution rights are not certified.",
        m.name, m.version, m.license, m.source, m.weights_sha256
    );
    let output = if task.joints() > 0 {
        Output {
            task,
            image: None,
            points: floats.as_chunks::<3>().0.to_vec(),
            report,
        }
    } else {
        let w = input
            .width
            .checked_mul(m.scale as u32)
            .ok_or("Output size overflow")?;
        let h = input
            .height
            .checked_mul(m.scale as u32)
            .ok_or("Output size overflow")?;
        if w as usize * h as usize > core::MAX_OUTPUT {
            return Err("Inference output exceeds 4,194,304 pixels".into());
        }
        let mw = m.width * m.scale;
        let mh = m.height * m.scale;
        let ma = mw * mh;
        let mut pixels = Vec::with_capacity(w as usize * h as usize);
        let mut clipped = 0usize;
        for y in 0..h {
            for x in 0..w {
                let sx = ((x as f64 + 0.5) * mw as f64 / w as f64 - 0.5).clamp(0., mw as f64 - 1.);
                let sy = ((y as f64 + 0.5) * mh as f64 / h as f64 - 0.5).clamp(0., mh as f64 - 1.);
                let ix = sx.floor() as usize;
                let iy = sy.floor() as usize;
                let fx = (sx - ix as f64) as f32;
                let fy = (sy - iy as f64) as f32;
                let mut p = [0u8; 4];
                for c in 0..3 {
                    let at = |xx: usize, yy: usize| floats[c * ma + yy * mw + xx];
                    let a = at(ix, iy) * (1. - fx) + at((ix + 1).min(mw - 1), iy) * fx;
                    let b = at(ix, (iy + 1).min(mh - 1)) * (1. - fx)
                        + at((ix + 1).min(mw - 1), (iy + 1).min(mh - 1)) * fx;
                    let v = a * (1. - fy) + b * fy;
                    if !(0. ..=1.).contains(&v) {
                        clipped += 1;
                    }
                    p[c] = (v.clamp(0., 1.) * 255.).round() as u8;
                }
                p[3] = (sample(
                    input,
                    (x as f64 + 0.5) / m.scale as f64 - 0.5,
                    (y as f64 + 0.5) / m.scale as f64 - 0.5,
                )[3] * 255.)
                    .round() as u8;
                if p[3] == 0 {
                    p = [0; 4];
                }
                pixels.push(p);
            }
        }
        Output {
            task,
            image: Some(LocalImage {
                width: w,
                height: h,
                pixels,
            }),
            points: vec![],
            report: format!(
                "{report} Output placed at native enlarged pixel size; canvas size is unchanged. RGB channels clipped: {clipped}."
            ),
        }
    };
    output.validate()?;
    Ok(output)
}
pub fn helper(directory: &Path) -> Result<(), String> {
    let request: Request =
        serde_json::from_slice(&read_bytes(&directory.join("request.json"), 16384)?)
            .map_err(|e| e.to_string())?;
    let result = std::panic::catch_unwind(|| {
        let input = core::read_image(&directory.join("input.rgba"), core::MAX_INPUT)?;
        let hints = if request.task == Task::HintColor {
            Some(core::read_image(
                &directory.join("hints.rgba"),
                core::MAX_INPUT,
            )?)
        } else {
            None
        };
        run(
            Path::new(&request.manifest),
            request.task,
            &input,
            hints.as_ref(),
        )
    })
    .unwrap_or_else(|_| {
        Err("The local inference engine rejected this model (panic); original unchanged".into())
    });
    let reply = match result {
        Ok(out) => {
            if let Some(i) = out.image {
                core::write_image(&directory.join("output.rgba"), &i, core::MAX_OUTPUT)?;
            }
            Reply {
                report: out.report,
                points: out.points,
                error: None,
            }
        }
        Err(e) => Reply {
            report: String::new(),
            points: vec![],
            error: Some(e.chars().take(8000).collect()),
        },
    };
    std::fs::write(
        directory.join("reply.json"),
        serde_json::to_vec(&reply).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
