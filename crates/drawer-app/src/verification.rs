//! Owned, deterministic acceptance scenarios. No physical-device claims.
use std::{
    io::{Read, Write},
    path::PathBuf,
};
#[path = "verification_capture.rs"]
mod capture;
#[path = "verification_coordinates.rs"]
mod coordinates;
#[path = "verification_editor.rs"]
mod editor;
#[path = "verification_history.rs"]
mod history;
#[path = "verification_masks.rs"]
mod masks;
#[path = "verification_native.rs"]
mod native;
#[path = "verification_recovery.rs"]
mod recovery;
#[path = "verification_strokes.rs"]
mod strokes;
pub(super) fn cli(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let item = args
        .next()
        .ok_or("Missing checklist number")?
        .to_str()
        .ok_or("Invalid checklist number")?
        .parse::<u32>()
        .map_err(|_| "Invalid checklist number")?;
    if !matches!(item, 287..=293) {
        return Err("This acceptance number has no implemented scenario".into());
    }
    let root = PathBuf::from(args.next().ok_or("Missing new verification directory")?);
    if args.next().is_some() {
        return Err("Unexpected verification arguments".into());
    }
    let root = drawer_core::file_browser::new_path(&root)?;
    std::fs::create_dir(&root)
        .map_err(|e| format!("Choose a NEW local verification directory: {e}"))?;
    let marker = serde_json::json!({"format":"Drawer owned verification","version":1,"item":item,"pid":std::process::id()});
    write(&root.join("owner.json"), &marker)?;
    let report = match item {
        287 => strokes::run(&root)?,
        288 => coordinates::run()?,
        289 => capture::run(&root)?,
        290 => masks::run(&root)?,
        291 => history::run(&root)?,
        292 => native::run(&root)?,
        293 => recovery::run(&root)?,
        _ => unreachable!(),
    };
    let mut sample = drawer_core::document::Document::create(
        drawer_core::document::CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: drawer_core::document::Background::Transparent,
        },
        drawer_core::document::DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let native_path = root.join("native-format-sample.drawer");
    drawer_core::files::save(&mut sample, &native_path).map_err(|e| e.to_string())?;
    let mut native = [0u8; 12];
    std::fs::File::open(native_path)
        .and_then(|mut f| f.read_exact(&mut native))
        .map_err(|e| e.to_string())?;
    if &native[..8] != b"DRAWER\r\n" {
        return Err("Invalid owned Native magic".into());
    }
    let version = u32::from_le_bytes(
        native
            .get(8..12)
            .ok_or("Missing owned Native header")?
            .try_into()
            .map_err(|_| "Invalid owned Native header")?,
    );
    write(
        &root.join("receipt.json"),
        &serde_json::json!({
            "format":"Drawer checklist acceptance","version":1,"item":item,"passed":true,
            "appVersion":env!("CARGO_PKG_VERSION"),"nativeVersion":version,
            "physicalDeviceValidated":false,"report":report,
        }),
    )
}
fn write(path: &std::path::Path, value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}

pub(super) fn native_child(
    mut args: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(), String> {
    let root = std::path::PathBuf::from(args.next().ok_or("Missing native verification root")?);
    let expected = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or("Missing native manifest hash")?;
    if args.next().is_some() {
        return Err("Unexpected native verification arguments".into());
    }
    native::child(&root, &expected)
}

pub(super) fn crash_child(
    mut args: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(), String> {
    let root = std::path::PathBuf::from(args.next().ok_or("Missing crash verification root")?);
    let expected = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or("Missing crash manifest hash")?;
    if args.next().is_some() {
        return Err("Unexpected crash verification arguments".into());
    }
    recovery::worker(&root, &expected)
}
