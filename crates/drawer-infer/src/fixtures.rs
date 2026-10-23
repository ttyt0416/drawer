//! Owned deterministic adapter fixtures, not trained models or quality endorsements.
use super::*;
fn tensor(name: &str, dims: &[i64], values: Vec<f32>) -> pb::TensorProto {
    pb::TensorProto {
        name: name.into(),
        dims: dims.to_vec(),
        data_type: 1,
        float_data: values,
        ..Default::default()
    }
}
fn ints(name: &str, values: Vec<i64>) -> pb::TensorProto {
    pb::TensorProto {
        name: name.into(),
        dims: vec![values.len() as i64],
        data_type: 7,
        int64_data: values,
        ..Default::default()
    }
}
fn value(name: &str, shape: &[i64]) -> pb::ValueInfoProto {
    pb::ValueInfoProto {
        name: name.into(),
        r#type: Some(pb::TypeProto {
            value: Some(pb::type_proto::Value::TensorType(pb::type_proto::Tensor {
                elem_type: 1,
                shape: Some(pb::TensorShapeProto {
                    dim: shape
                        .iter()
                        .map(|n| pb::tensor_shape_proto::Dimension {
                            value: Some(pb::tensor_shape_proto::dimension::Value::DimValue(*n)),
                            ..Default::default()
                        })
                        .collect(),
                }),
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn node(op: &str, inputs: &[&str], outputs: &[&str]) -> pb::NodeProto {
    pb::NodeProto {
        op_type: op.into(),
        input: inputs.iter().map(|s| (*s).into()).collect(),
        output: outputs.iter().map(|s| (*s).into()).collect(),
        ..Default::default()
    }
}
pub fn input() -> LocalImage {
    LocalImage {
        width: 2,
        height: 2,
        pixels: vec![
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 128],
            [250, 200, 100, 0],
        ],
    }
}
pub fn hints() -> LocalImage {
    LocalImage {
        width: 2,
        height: 2,
        pixels: vec![[0, 0, 255, 255]; 4],
    }
}
pub fn bundle(root: &Path, task: Task) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let channels = if task == Task::HintColor { 7 } else { 3 };
    let scale = if task == Task::Upscale { 2 } else { 1 };
    let mut g = pb::GraphProto {
        input: vec![value("input", &[1, channels, 2, 2])],
        ..Default::default()
    };
    if task.joints() > 0 {
        let n = task.joints() * 3;
        let mut weights = vec![0.; 12 * n];
        let mut bias = vec![0.; n];
        for j in 0..task.joints() {
            weights[j * 3] = 0.01;
            bias[j * 3] = (j % 5) as f32 / 6.;
            bias[j * 3 + 1] = (j / 5) as f32 / 6.;
            bias[j * 3 + 2] = if j == 0 { 0.1 } else { 0.9 };
        }
        g.initializer = vec![
            tensor("w", &[12, n as i64], weights),
            tensor("b", &[n as i64], bias),
            ints("shape", vec![1, task.joints() as i64, 3]),
        ];
        g.node = vec![
            node("Flatten", &["input"], &["flat"]),
            node("Gemm", &["flat", "w", "b"], &["points"]),
            node("Reshape", &["points", "shape"], &["output"]),
        ];
        g.output = vec![value("output", &[1, task.joints() as i64, 3])];
    } else if task == Task::Upscale {
        g.initializer = vec![
            tensor("scales", &[4], vec![1., 1., 2., 2.]),
            tensor("roi", &[0], vec![]),
        ];
        let mut n = node("Resize", &["input", "roi", "scales"], &["output"]);
        n.attribute.push(pb::AttributeProto {
            name: "mode".into(),
            r#type: 3,
            s: b"nearest".to_vec(),
            ..Default::default()
        });
        for (name, text) in [
            ("coordinate_transformation_mode", "asymmetric"),
            ("nearest_mode", "floor"),
        ] {
            n.attribute.push(pb::AttributeProto {
                name: name.into(),
                r#type: 3,
                s: text.as_bytes().to_vec(),
                ..Default::default()
            });
        }
        g.node = vec![n];
        g.output = vec![value("output", &[1, 3, 4, 4])];
    } else {
        let kernel = if task == Task::Denoise { 3 } else { 1 };
        let mut w = vec![0.; 3 * channels as usize * kernel * kernel];
        for c in 0..3 {
            for k in 0..kernel * kernel {
                w[(c * channels as usize + c) * kernel * kernel + k] =
                    if task == Task::Denoise { 1. / 9. } else { 0.5 };
            }
            if task == Task::HintColor {
                w[c * channels as usize + c + 3] = 0.25;
                w[c * channels as usize + 6] = 0.25;
            }
        }
        g.initializer = vec![tensor("w", &[3, channels, kernel as i64, kernel as i64], w)];
        let mut n = node("Conv", &["input", "w"], &["output"]);
        if kernel == 3 {
            n.attribute.push(pb::AttributeProto {
                name: "pads".into(),
                r#type: 7,
                ints: vec![1; 4],
                ..Default::default()
            });
        }
        g.node = vec![n];
        g.output = vec![value("output", &[1, 3, 2, 2])];
    }
    let proto = pb::ModelProto {
        ir_version: 7,
        opset_import: vec![pb::OperatorSetIdProto {
            domain: String::new(),
            version: 11,
        }],
        graph: Some(g),
        ..Default::default()
    };
    let weights = proto.encode_to_vec();
    let license=b"MIT License\nCopyright (c) 2026 Drawer contributors\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the Software), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.\nTHE SOFTWARE IS PROVIDED AS IS, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.\n";
    let m = Manifest {
        format: 1,
        name: "Adapter algebra fixture (NOT trained)".into(),
        version: "1".into(),
        license: "MIT".into(),
        source: "Drawer-owned deterministic algebra / fixtures.rs".into(),
        weights_sha256: hash(&weights),
        license_sha256: hash(license),
        task,
        width: 2,
        height: 2,
        scale,
        mean: [0.; 3],
        std: [1.; 3],
    };
    std::fs::write(root.join("weights.onnx"), weights).map_err(|e| e.to_string())?;
    std::fs::write(root.join("LICENSE.txt"), license).map_err(|e| e.to_string())?;
    let path = root.join("manifest.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(path)
}
