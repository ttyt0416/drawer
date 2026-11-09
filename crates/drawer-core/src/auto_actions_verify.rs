use crate::{
    auto_actions::*,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    storage::UserStorage,
};
use std::{path::Path, sync::atomic::AtomicBool};
pub fn fixture() -> Result<(Settings, u64, u64), String> {
    let mut s = Settings::default();
    let set = s.add_set("Local set".into())?;
    let a = s.add_action(set, "Fill and name".into())?;
    s.append(
        a,
        Operation::Fill {
            layer: 0,
            rgba: [210, 45, 80, 255],
            erase: false,
            alpha_lock: false,
        },
    )?;
    s.append(
        a,
        Operation::RenameRaster {
            layer: 0,
            name: "Recorded".into(),
        },
    )?;
    Ok((s, set, a))
}
pub(crate) fn editing() -> Result<(), String> {
    let (mut s, set, a) = fixture()?;
    let old = s.clone();
    let b = s.duplicate_action(a)?;
    if s.action(a) != old.action(a)
        || s.action(a).unwrap().steps[0].id == s.action(b).unwrap().steps[0].id
    {
        return Err("Action self-copy reused IDs or changed source".into());
    }
    s.duplicate_step(a, 0)?;
    s.action_mut(a)?.steps.swap(0, 2);
    s.action_mut(a)?.steps.remove(1);
    let c = s.duplicate_set(set)?;
    if c == set || !s.valid() {
        return Err("Set copy invalid".into());
    }
    let imported = s.import(&old)?;
    if imported.sets.len() != 3 || imported.sets[2].id == old.sets[0].id || s.sets.len() != 2 {
        return Err("Import changed source or reused IDs".into());
    }
    let before = s.clone();
    if s.add_action(set, "\n".into()).is_ok() || s != before {
        return Err("Invalid edit was partial".into());
    }
    s.sets[0].actions.clear();
    if !s.valid() {
        return Err("Empty set / last action delete rejected".into());
    }
    s.sets.clear();
    if !s.valid() {
        return Err("Last set delete rejected".into());
    }
    Ok(())
}
fn document() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 4,
            height: 3,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn playback() -> Result<(), String> {
    let (mut s, _, a) = fixture()?;
    let mut d = document()?;
    let original = d.snapshot();
    let off = AtomicBool::new(false);
    let p = prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off)?;
    if d.snapshot() != original || d.history_counts() != (0, 0) {
        return Err("Preparation edited document".into());
    }
    if !p.commit(&mut d)?
        || d.layers[0].name != "Recorded"
        || d.layers[0].pixel(4, 3, 0, 0) != [210, 45, 80, 255]
        || d.history_counts() != (1, 0)
    {
        return Err("Parameterized playback or grouped undo failed".into());
    }
    let result = d.snapshot();
    if !d.undo() || d.snapshot() != original || !d.redo() || d.snapshot() != result {
        return Err("Action Undo/Redo did not restore complete state".into());
    }
    s.append(
        a,
        Operation::Opacity {
            layer: 255,
            percent: 50,
        },
    )?;
    if prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off).is_ok()
        || d.snapshot() != result
    {
        return Err("Late step failure was partial".into());
    }
    s.action_mut(a)?.steps.pop();
    let pending = prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off)?;
    d.revision += 1;
    if pending.commit(&mut d).is_ok() {
        return Err("Stale replay accepted".into());
    }
    let pending = prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off)?;
    d.current_frame += 1;
    if pending.commit(&mut d).is_ok() {
        return Err("Changed frame accepted".into());
    }
    d.current_frame = 0;
    if prepare(
        &d,
        s.action(a).unwrap(),
        DEFAULT_DOCUMENT_BUDGET,
        &AtomicBool::new(true),
    )
    .is_ok()
    {
        return Err("Canceled playback accepted".into());
    }
    if prepare(&d, s.action(a).unwrap(), 0, &off).is_ok() {
        return Err("Memory budget ignored".into());
    }
    d.layers[0].locked = true;
    if prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off).is_ok() {
        return Err("Locked target accepted".into());
    }
    d.layers[0].locked = false;
    let mut other = document()?;
    let p = prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off)?;
    if p.commit(&mut other).is_ok() {
        return Err("Different document accepted".into());
    }
    let mut empty = s.action(a).unwrap().clone();
    empty.steps.clear();
    if prepare(&d, &empty, DEFAULT_DOCUMENT_BUDGET, &off).is_ok() {
        return Err("Empty playback accepted".into());
    }
    s.append(
        a,
        Operation::Unsupported {
            command: "shell".into(),
            tool: None,
        },
    )?;
    if prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off).is_ok() {
        return Err("Untrusted command executed".into());
    }
    s.action_mut(a)?.steps.last_mut().unwrap().enabled = false;
    prepare(&d, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off)?;
    // Ordinal targets resolve in another document; no captured native layer identity is used.
    let (s, _, a) = fixture()?;
    prepare(&other, s.action(a).unwrap(), DEFAULT_DOCUMENT_BUDGET, &off)?.commit(&mut other)?;
    // Existing 16-bit fill pipeline must keep its precise buffer.
    let mut precise = Document::create_with_color(
        CanvasSpec {
            width: 3,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        crate::color_storage::Config {
            depth: crate::color_precision::Depth::Sixteen,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let state = prepare_operation(
        &precise,
        &Operation::Fill {
            layer: 0,
            rgba: [20, 40, 60, 128],
            erase: false,
            alpha_lock: false,
        },
        &off,
    )?;
    precise.commit_state(state)?;
    if precise.layers[0].precise.is_none() {
        return Err("Auto action lost 16-bit data".into());
    }
    Ok(())
}
pub(crate) fn variants() -> Result<(), String> {
    let mut d = Document::create_with_color(
        CanvasSpec {
            width: 4,
            height: 3,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
        crate::color_storage::Config {
            depth: crate::color_precision::Depth::Sixteen,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let original = d.snapshot();
    let mut s = Settings::default();
    let set = s.add_set("Set".into())?;
    let a = s.add_action(set, "🎨".repeat(64))?;
    for op in [
        Operation::NewRaster {
            name: "Second".into(),
        },
        Operation::Fill {
            layer: 1,
            rgba: [20, 60, 110, 255],
            erase: false,
            alpha_lock: false,
        },
        Operation::Opacity {
            layer: 1,
            percent: 37,
        },
        Operation::Dpi(144.),
        Operation::Canvas(Canvas::Clockwise),
        Operation::Canvas(Canvas::FlipHorizontal),
        Operation::Canvas(Canvas::Counterclockwise),
        Operation::Canvas(Canvas::FlipVertical),
    ] {
        s.append(a, op)?;
    }
    let p = prepare(
        &d,
        s.action(a).unwrap(),
        DEFAULT_DOCUMENT_BUDGET,
        &AtomicBool::new(false),
    )?;
    if p.label.len() > 256 || d.snapshot() != original {
        return Err("Unicode history label or isolated preparation failed".into());
    }
    p.commit(&mut d)?;
    if (d.width, d.height, d.dpi, d.layers.len()) != (4, 3, 144., 2)
        || d.layers[1].name != "Second"
        || d.layers[1].precise.is_none()
        || d.layers[1].pixel(4, 3, 0, 0) != [20, 60, 110, 255]
        || crate::layer_properties::properties(&d, crate::layer_properties::raster(&d, 1)).opacity
            != 37
    {
        return Err("Canvas / DPI / layer / fill / opacity sequence lost state".into());
    }
    if !d.undo() || d.snapshot() != original {
        return Err("Compound action did not undo once".into());
    }
    let hidden = crate::layer_properties::prepare_patch(
        &d,
        &[crate::layer_properties::raster(&d, 0)],
        crate::layer_properties::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(hidden)?;
    if prepare_operation(
        &d,
        &Operation::Fill {
            layer: 0,
            rgba: [0, 0, 0, 255],
            erase: false,
            alpha_lock: false,
        },
        &AtomicBool::new(false),
    )
    .is_ok()
    {
        return Err("Hidden action target accepted".into());
    }
    Ok(())
}
pub(crate) fn files(root: &Path) -> Result<(), String> {
    let (mut s, _, a) = fixture()?;
    let path = root.join("actions.json");
    s.write(&path, false)?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    if Settings::read(&path)? != s
        || s.write(&path, false).is_ok()
        || std::fs::read(&path).map_err(|e| e.to_string())? != bytes
    {
        return Err("Action exchange or overwrite guard failed".into());
    }
    for bytes in [
        b"{bad".as_slice(),
        b"{}".as_slice(),
        &bytes[..bytes.len() - 1],
    ] {
        if Settings::decode(bytes).is_ok() {
            return Err("Damaged action file accepted".into());
        }
    }
    let mut invalid = s.clone();
    invalid.version = 9;
    if Settings::decode(&serde_json::to_vec(&invalid).unwrap()).is_ok() {
        return Err("Unknown version accepted".into());
    }
    invalid = s.clone();
    invalid.sets[0].actions[0].steps[0].id = invalid.sets[0].id;
    if Settings::decode(&serde_json::to_vec(&invalid).unwrap()).is_ok() {
        return Err("Duplicate IDs accepted".into());
    }
    let raw = String::from_utf8(bytes.clone())
        .unwrap()
        .replace("Fill\"", "Shell\"");
    if Settings::decode(raw.as_bytes()).is_ok() {
        return Err("Unknown operation accepted".into());
    }
    s.append(
        a,
        Operation::Unsupported {
            command: "local-tool".into(),
            tool: Some(ToolRef {
                document: 99,
                id: 7,
            }),
        },
    )?;
    if !s.issues(Some(1), &[7])[0].contains("another document")
        || !s.issues(Some(99), &[])[0].contains("missing tool")
    {
        return Err("Foreign / missing tool diagnostics absent".into());
    }
    s.mode = Mode::Buttons;
    let storage = UserStorage::at(root.join("user")).map_err(|e| e.to_string())?;
    s.save(&storage)?;
    if Settings::load(&storage)? != s {
        return Err("Action settings restart failed".into());
    }
    let mut temp = path.file_name().unwrap().to_owned();
    temp.push(".actions-write.tmp");
    let temp = path.with_file_name(temp);
    std::fs::write(&temp, b"foreign").map_err(|e| e.to_string())?;
    if s.write(&path, true).is_ok()
        || std::fs::read(&temp).unwrap() != b"foreign"
        || std::fs::read(&path).unwrap() != bytes
    {
        return Err("Foreign temporary or old action file changed".into());
    }
    std::fs::remove_file(temp).map_err(|e| e.to_string())?;
    s.write(&path, true)?;
    Ok(())
}
pub fn verify(root: &Path) -> Result<(), String> {
    let dir = root.join("auto-actions-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = editing()
        .and_then(|_| playback())
        .and_then(|_| variants())
        .and_then(|_| files(&dir));
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
