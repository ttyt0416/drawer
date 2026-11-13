use super::editor::check;
use drawer_core::{
    asset_catalog::hash,
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    scene::{PageObject, Rect},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    label: String,
    file: String,
    sha256: String,
    bytes: usize,
    probes: String,
    frame: u32,
    scene_fields: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    parent: u32,
    cases: Vec<Case>,
}
pub(super) fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = drawer_core::file_browser::checked(path)?;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    check(
        bytes.len() <= limit,
        "Owned verification data exceeds its size limit",
    )?;
    Ok(bytes)
}
pub(super) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())
}
fn probes(d: &Document) -> Result<String, String> {
    let r = drawer_core::render::Renderer::new(d);
    let mut bytes = vec![];
    for y in 0..9 {
        for x in 0..9 {
            let p = [
                (f64::from(x) + 0.5) * f64::from(d.width) / 9.,
                (f64::from(y) + 0.5) * f64::from(d.height) / 9.,
            ];
            bytes.extend(r.sample(p)?.rgba8());
        }
    }
    Ok(hash(&bytes))
}
fn capture(dir: &Path, label: &str, mut d: Document, cases: &mut Vec<Case>) -> Result<(), String> {
    let mut s = d.snapshot();
    s.layers[0].name = format!("숨긴 원본 🎨 {label}");
    d.commit_state(s)?;
    let target = drawer_core::layer_properties::raster(&d, 0);
    let s = drawer_core::layer_properties::prepare_patch(
        &d,
        &[target],
        drawer_core::layer_properties::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(s)?;
    if let Some(a) = &d.animation {
        d.current_frame = a.frames - 1;
    }
    let before = d.snapshot();
    let frame = d.current_frame;
    let view = d.view;
    let expected = probes(&d)?;
    let scene = serde_json::to_value(&d.scene).map_err(|e| e.to_string())?;
    let fields = scene
        .as_object()
        .ok_or("Scene is not an object")?
        .keys()
        .cloned()
        .collect();
    let file = format!("문서-{:03}.drawer", cases.len());
    let path = dir.join(&file);
    drawer_core::files::save(&mut d, &path).map_err(|e| format!("{label}: {e}"))?;
    check(
        d.snapshot() == before,
        "Native save changed the original semantic state",
    )?;
    drop(d);
    let reopened = drawer_core::files::open(&path, B).map_err(|e| format!("{label}: {e}"))?;
    check(
        reopened.snapshot() == before
            && reopened.current_frame == frame
            && probes(&reopened)? == expected,
        &format!("{label}: save/close/reopen changed state, frame or rendered probes"),
    )?;
    // View is excluded from the D comparison; no assertion that opening restores it.
    let _ = view;
    drop(reopened);
    let bytes = read(&path, 32 * 1024 * 1024)?;
    cases.push(Case {
        label: label.into(),
        file,
        sha256: hash(&bytes),
        bytes: bytes.len(),
        probes: expected,
        frame,
        scene_fields: fields,
    });
    Ok(())
}
fn precise() -> Result<Document, String> {
    let mut d = Document::create_with_color(
        drawer_core::document::CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.,
            background: drawer_core::document::Background::Transparent,
        },
        B,
        drawer_core::color_storage::Config {
            depth: drawer_core::color_precision::Depth::Sixteen,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let data = vec![
        [0, 0, 0, 0],
        [1234, 34567, 65000, 32768],
        [65535, 12345, 40000, 65535],
    ];
    d.layers[0].pixels = data
        .iter()
        .map(|p| drawer_core::color_precision::Sample::from16(*p).rgba8())
        .collect();
    d.layers[0].precise = Some(data);
    Ok(d)
}
fn sparse() -> Result<Document, String> {
    let mut d = Document::create(
        drawer_core::document::CanvasSpec {
            width: 130,
            height: 4,
            dpi: 300.,
            background: drawer_core::document::Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let mut t = drawer_core::tiled_raster::Image::new(130, 4, [0; 4]);
    t.prepare([63, 64, 127, 128].into_iter(), B)?;
    for n in [63, 64, 127, 128] {
        t.set_prepared(n, [n as u8, 30, 80, 128]);
    }
    d.layers[0].tiled = Some(t);
    d.layers[0].pixels.clear();
    Ok(d)
}
fn objects() -> Result<Document, String> {
    let mut d = drawer_core::text_version_verify::fixture()?;
    for object in drawer_core::comics::defaults(Rect {
        min: [70., 10.],
        max: [110., 50.],
    }) {
        let s = drawer_core::comics::prepare(&d, None, object)?;
        d.commit_state(s)?;
    }
    let material = drawer_core::material_layer::Layer {
        source: drawer_core::material_layer::Source::Solid([12, 34, 56, 128]),
        area: Rect {
            min: [0.; 2],
            max: [8.; 2],
        },
        transform: Default::default(),
        repeat: [false; 2],
        mapping: Default::default(),
    };
    let (s, _) = drawer_core::material_layer::prepare(&d, None, "로컬 채움", material)?;
    d.commit_state(s)?;
    let mut s = d.snapshot();
    s.scene.objects.extend([
        PageObject::Line {
            start: [1.5, 1.5],
            end: [1.5, 1.5],
            width_px: 1.,
            color: [10, 20, 30, 128],
        },
        PageObject::Tone {
            bounds: Rect {
                min: [120., 10.],
                max: [150., 40.],
            },
            period_px: 6.,
            coverage: 0.5,
        },
        PageObject::Shadow {
            bounds: Rect {
                min: [160., 10.],
                max: [180., 30.],
            },
            offset: [2., 3.],
            radius_px: 3.,
            color: [0, 0, 0, 128],
        },
    ]);
    d.commit_state(s)?;
    Ok(d)
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let dir = root.join("한글 네이티브 🎨");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let mut cases = vec![];
    type Fixture = fn() -> Result<Document, String>;
    let fixtures: [(&str, Fixture); 15] = [
        ("page-objects-all-variants", objects),
        (
            "raster-selection",
            drawer_core::affine_transform_verify::fixture,
        ),
        ("folders-marks", drawer_core::fill_reference_verify::fixture),
        ("masks", drawer_core::layer_masks_verify::fixture),
        (
            "texture-ruler-vector",
            drawer_core::layer_groups_verify::fixture,
        ),
        ("editable-vector", drawer_core::vector_edit_verify::fixture),
        ("vector-scope", drawer_core::vector_scope_verify::fixture),
        ("line-tools", drawer_core::line_tools_verify::fixture),
        (
            "color-corrections",
            drawer_core::color_corrections_verify::fixture,
        ),
        (
            "surface-effects",
            drawer_core::surface_effects_verify::fixture,
        ),
        ("rgba16", precise),
        ("world3d", drawer_core::world3d_verify::fixture),
        ("lighting", drawer_core::lighting_verify::fixture),
        ("camera", drawer_core::camera_library_verify::fixture),
        ("world-views", drawer_core::world_views_verify::fixture),
    ];
    for (label, fixture) in fixtures {
        capture(&dir, label, fixture()?, &mut cases)?;
    }
    capture(&dir, "sparse-tiles", sparse()?, &mut cases)?;
    capture(
        &dir,
        "timeline-cels",
        drawer_core::animation_export_verify::fixture(),
        &mut cases,
    )?;
    capture(
        &dir,
        "timeline-pcm-timing",
        drawer_core::timeline_timing_verify::fixture(),
        &mut cases,
    )?;
    capture(
        &dir,
        "keyframes",
        drawer_core::keyframes_verify::fixture(),
        &mut cases,
    )?;
    capture(
        &dir,
        "pose-sequence",
        drawer_core::pose_sequence_verify::fixture()?.0,
        &mut cases,
    )?;
    let mut linked = drawer_core::text_layout_verify::fixture()?;
    let fonts = drawer_core::font_library_verify::fixture(&dir)?;
    let font = fonts
        .entries
        .iter()
        .find(|e| {
            matches!(e.font, drawer_core::text_layout::Font::Local(_))
                && e.name.to_lowercase().contains("ubuntu")
        })
        .ok_or("Owned local font missing")?;
    let mut settings = drawer_core::text_layout_verify::settings();
    settings.text = "AB 123".into();
    settings.style.font = font.font;
    linked.commit_state(drawer_core::text_layout::prepare(&linked, None, &settings)?.state)?;
    let source = drawer_core::file_sources_verify::source(&dir)?;
    let (s, _) = drawer_core::file_objects::create(&linked, &source, "Linked cache")?;
    linked.commit_state(s)?;
    let model = drawer_core::model_formats_verify::fixture(&dir.join("model-source"), "vrm")?;
    drawer_core::model_library::prepare(
        &linked,
        drawer_core::model_library::Edit::Import {
            path: model.clone(),
            placement: drawer_core::model_library::Placement {
                fit: None,
                ..Default::default()
            },
        },
        B,
        &AtomicBool::new(false),
    )?
    .commit(&mut linked)?;
    capture(&dir, "missing-font-page-model-caches", linked, &mut cases)?;
    for p in [
        font.path.as_ref().ok_or("Owned font has no path")?,
        &source,
        &model,
    ] {
        std::fs::remove_file(p).map_err(|e| e.to_string())?;
    }
    drawer_core::text_layout::invalidate_font_data();
    let manifest = Manifest {
        format: "Drawer native roundtrip".into(),
        version: 1,
        parent: std::process::id(),
        cases,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    write(&dir.join("manifest.json"), &bytes)?;
    let expected = hash(&bytes);
    if cfg!(test) {
        child(&dir, &expected)?;
    } else {
        let mut command =
            std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        command
            .arg("--verify-native-roundtrip")
            .arg(&dir)
            .arg(&expected)
            .env("DRAWER_DATA_DIR", dir.join("fresh-profile"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut process = command.spawn().map_err(|e| e.to_string())?;
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = process.try_wait().map_err(|e| e.to_string())? {
                check(status.success(), "Separate native roundtrip process failed")?;
                break;
            }
            if start.elapsed() > std::time::Duration::from_secs(60) {
                let _ = process.kill();
                let _ = process.wait();
                return Err("Separate native roundtrip process timed out".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let receipt: serde_json::Value =
        serde_json::from_slice(&read(&dir.join("child-receipt.json"), 1024 * 1024)?)
            .map_err(|e| e.to_string())?;
    check(
        receipt["passed"] == true
            && receipt["manifestSha256"] == expected
            && receipt["cases"] == manifest.cases.len(),
        "Native child receipt differs",
    )?;
    Ok(
        serde_json::json!({"documents":manifest.cases,"separateProcess":!cfg!(test),"manifestSha256":expected,"childReceiptSha256":hash(&serde_json::to_vec(&receipt).map_err(|e|e.to_string())?),"semanticStateAndFrameExact":true,"canonicalNativeBytesExact":true,"renderedProbeGrid":[9,9],"originalResourceFilesDeleted":true,"separateOfflineComputerValidated":false}),
    )
}
pub(super) fn child(root: &Path, expected: &str) -> Result<(), String> {
    check(
        drawer_core::asset_catalog::valid_hash(expected),
        "Invalid native manifest hash",
    )?;
    let root = drawer_core::file_browser::folder(root)?;
    let bytes = read(&root.join("manifest.json"), 1024 * 1024)?;
    check(hash(&bytes) == expected, "Native manifest changed")?;
    let m: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    check(
        m.format == "Drawer native roundtrip"
            && m.version == 1
            && m.parent > 0
            && !m.cases.is_empty()
            && m.cases.len() <= 32
            && m.cases.iter().map(|c| c.bytes).sum::<usize>() <= 128 * 1024 * 1024,
        "Invalid native manifest limits",
    )?;
    let output = root.join("reencoded");
    std::fs::create_dir(&output).map_err(|e| e.to_string())?;
    for (n, c) in m.cases.iter().enumerate() {
        check(
            c.file == format!("문서-{n:03}.drawer")
                && c.bytes <= 32 * 1024 * 1024
                && drawer_core::asset_catalog::valid_hash(&c.sha256)
                && drawer_core::asset_catalog::valid_hash(&c.probes),
            "Invalid native case identity",
        )?;
        let path = root.join(&c.file);
        let bytes = read(&path, 32 * 1024 * 1024)?;
        check(
            bytes.len() == c.bytes && hash(&bytes) == c.sha256,
            "Native case changed",
        )?;
        let mut d = drawer_core::files::open(&path, B).map_err(|e| format!("{}: {e}", c.label))?;
        check(
            d.current_frame == c.frame && probes(&d)? == c.probes,
            &format!("{}: separate reopen changed frame/appearance", c.label),
        )?;
        let second = output.join(&c.file);
        drawer_core::files::save(&mut d, &second).map_err(|e| e.to_string())?;
        check(
            read(&second, 32 * 1024 * 1024)? == bytes,
            &format!(
                "{}: canonical native data differs after separate reopen",
                c.label
            ),
        )?;
    }
    write(&root.join("child-receipt.json"),&serde_json::to_vec(&serde_json::json!({"version":1,"passed":true,"manifestSha256":expected,"cases":m.cases.len()})).map_err(|e|e.to_string())?)
}
#[cfg(test)]
mod tests {
    #[test]
    fn typed_native_documents_hidden_unicode_frames_resources_and_fresh_reencoding() {
        let r = crate::tests::TestDirectory::new();
        super::run(&r.0).unwrap();
    }
}
