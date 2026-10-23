use super::*;
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = loop {
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "drawer-model-verify-{}-{}-{serial}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("Cannot create model verification directory: {e}"),
            }
        };
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn real_onnx_adapters_fixed_inputs_hints_pose_alpha_and_atomic_adoption() {
    let root = Temp::new();
    let input = fixtures::input();
    let hints = fixtures::hints();
    for task in Task::ALL {
        let path = fixtures::bundle(&root.0.join(format!("{task:?}")), task).unwrap();
        let out = run(&path, task, &input, Some(&hints)).unwrap();
        out.validate().unwrap();
        assert!(out.report.contains("not certified"));
        if task.joints() > 0 {
            assert_eq!(out.points.len(), task.joints());
            assert!((out.points[0][0] - 0.01).abs() < 1e-6);
            assert!(out.low_confidence(0.5));
            let mut changed = input.clone();
            changed.pixels[0] = [0, 0, 0, 255];
            let other = run(&path, task, &changed, None).unwrap();
            assert!(other.points[0][0] < out.points[0][0]);
        } else {
            let image = out.image.as_ref().unwrap();
            assert_eq!(image.width, if task == Task::Upscale { 4 } else { 2 });
            assert_eq!(image.pixels.last().unwrap(), &[0; 4]);
            assert_eq!(image.pixels[0][3], 255);
            if task == Task::HintColor {
                assert_eq!(image.pixels[0], [191, 64, 128, 255]);
                let mut h = hints.clone();
                h.pixels.fill([0, 255, 0, 255]);
                assert_ne!(
                    run(&path, task, &input, Some(&h))
                        .unwrap()
                        .image
                        .unwrap()
                        .pixels,
                    image.pixels
                );
            }
            if task == Task::Denoise {
                assert_eq!(image.pixels[0], image.pixels[1]);
                assert!(image.pixels[0][0] < 150);
            }
        }
        let mut d = drawer_core::document::Document::create(
            drawer_core::document::CanvasSpec {
                width: 2,
                height: 2,
                dpi: 300.,
                background: drawer_core::document::Background::Transparent,
            },
            drawer_core::document::DEFAULT_DOCUMENT_BUDGET,
        )
        .unwrap();
        let original = d.snapshot();
        let before = d.history_counts();
        let state = core::adopt(&d, &input, &out, 0.5, true).unwrap();
        assert_eq!(d.snapshot(), original);
        d.commit_state(state.clone()).unwrap();
        assert_eq!(d.history_counts().0, before.0 + 1);
        assert_eq!(d.layers[0], original.layers[0]);
        let native = root.0.join(format!("{task:?}.drawer"));
        drawer_core::files::save(&mut d, &native).unwrap();
        let reopened =
            drawer_core::files::open(&native, drawer_core::document::DEFAULT_DOCUMENT_BUDGET)
                .unwrap();
        assert_eq!(reopened.snapshot(), state);
        assert!(d.undo());
        assert_eq!(d.snapshot(), original);
        if task.joints() > 0 {
            assert!(core::adopt(&d, &input, &out, 0.5, false).is_err());
        }
    }
}
#[test]
fn missing_tampered_license_version_external_tensor_memory_and_invalid_results_are_rejected() {
    let root = Temp::new();
    let input = fixtures::input();
    let path = fixtures::bundle(&root.0, Task::Hand).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(
        run(&root.0.join("missing"), Task::Hand, &input, None)
            .unwrap_err()
            .contains("Install")
    );
    for field in [
        "format",
        "version",
        "license",
        "weights_sha256",
        "license_sha256",
        "width",
        "std",
    ] {
        let mut m: serde_json::Value = serde_json::from_slice(&original).unwrap();
        m[field] = match field {
            "format" => 2.into(),
            "width" => 999999.into(),
            "std" => serde_json::json!([0, 1, 1]),
            _ => "".into(),
        };
        std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(run(&path, Task::Hand, &input, None).is_err(), "{field}");
    }
    std::fs::write(&path, &original).unwrap();
    let weights = root.0.join("weights.onnx");
    std::fs::write(&weights, b"tampered").unwrap();
    assert!(
        run(&path, Task::Hand, &input, None)
            .unwrap_err()
            .contains("SHA-256")
    );
    fixtures::bundle(&root.0, Task::Hand).unwrap();
    let mut m: Manifest = serde_json::from_slice(&original).unwrap();
    let mut p = pb::ModelProto::decode(std::fs::read(&weights).unwrap().as_slice()).unwrap();
    p.graph.as_mut().unwrap().initializer[0]
        .external_data
        .push(pb::StringStringEntryProto {
            key: "location".into(),
            value: "https://invalid.example/model".into(),
        });
    let bytes = p.encode_to_vec();
    m.weights_sha256 = hash(&bytes);
    std::fs::write(&weights, bytes).unwrap();
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(
        run(&path, Task::Hand, &input, None)
            .unwrap_err()
            .contains("External")
    );
    let huge = pb::TensorProto {
        dims: vec![i64::MAX, i64::MAX],
        ..Default::default()
    };
    assert!(tensor(&huge).is_err());
    let mut transparent = input.clone();
    transparent.pixels.fill([255, 0, 0, 0]);
    assert!(run(&path, Task::Hand, &transparent, None).is_err());
    let bad = Output {
        task: Task::Hand,
        image: None,
        points: vec![[f32::NAN, 0., 0.]; 21],
        report: String::new(),
    };
    assert!(bad.validate().is_err());
    let path = fixtures::bundle(&root.0, Task::HintColor).unwrap();
    assert!(run(&path, Task::HintColor, &input, None).is_err());
}

#[test]
fn oversized_static_activations_are_rejected_before_inference_allocation() {
    let root = Temp::new();
    let path = fixtures::bundle(&root.0, Task::Denoise).unwrap();
    let mut m: Manifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m.width = 512;
    m.height = 512;
    let weights = root.0.join("weights.onnx");
    let mut p = pb::ModelProto::decode(std::fs::read(&weights).unwrap().as_slice()).unwrap();
    let g = p.graph.as_mut().unwrap();
    g.input[0].r#type.as_mut().unwrap().value =
        Some(pb::type_proto::Value::TensorType(pb::type_proto::Tensor {
            elem_type: 1,
            shape: Some(pb::TensorShapeProto {
                dim: [1, 3, 512, 512]
                    .into_iter()
                    .map(|n| pb::tensor_shape_proto::Dimension {
                        value: Some(pb::tensor_shape_proto::dimension::Value::DimValue(n)),
                        ..Default::default()
                    })
                    .collect(),
            }),
        }));
    g.output.clear();
    g.output.push(pb::ValueInfoProto {
        name: "output".into(),
        ..Default::default()
    });
    g.initializer[0] = pb::TensorProto {
        name: "w".into(),
        dims: vec![64, 3, 3, 3],
        data_type: 1,
        float_data: vec![0.; 64 * 3 * 9],
        ..Default::default()
    };
    let bytes = p.encode_to_vec();
    m.weights_sha256 = hash(&bytes);
    std::fs::write(weights, bytes).unwrap();
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    let error = run(&path, Task::Denoise, &fixtures::input(), None).unwrap_err();
    assert!(error.contains("memory limit"), "{error}");
}
