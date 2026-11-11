use crate::{
    document::{Background, CanvasSpec, Document},
    resource_plan::*,
};
pub fn run(root: &std::path::Path) -> Result<(), String> {
    for (w, h) in [(0, 1), (-1, 1), (1, 0), (32769, 1)] {
        if dense(w, h, 4, 1, 1).is_ok() {
            return Err("Invalid dimensions accepted".into());
        }
    }
    if bytes(u64::MAX, 2, 4, 1).is_ok() || dense(1, 1, 4, 1, u64::MAX).is_ok() {
        return Err("Overflow accepted".into());
    }
    let cost = dense(16, 16, 4, 1, 2)?;
    if cost.admit_memory(2047).is_ok()
        || cost.admit_memory(2048).is_err()
        || cost.admit_disk(cost.disk - 1).is_ok()
        || cost.admit_disk(cost.disk).is_err()
    {
        return Err("Budget boundary incorrect".into());
    }
    let mut d = Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.,
            background: Background::Transparent,
        },
        65536,
    )
    .map_err(|e| e.to_string())?;
    let path = root.join("resource-admission.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let estimate = crate::files::estimate_native_bytes(&d).map_err(|e| e.to_string())?;
    if estimate != bytes.len() as u64 || native_space(estimate, estimate)?.admit_disk(0).is_ok() {
        return Err("Native preflight incorrect".into());
    }
    // A 26-byte BMP advertises a huge decoded raster, without supplying that raster.
    let mut bomb = vec![0u8; 26];
    bomb[..2].copy_from_slice(b"BM");
    bomb[14..18].copy_from_slice(&12u32.to_le_bytes());
    bomb[18..20].copy_from_slice(&32768u16.to_le_bytes());
    bomb[20..22].copy_from_slice(&32768u16.to_le_bytes());
    bomb[22..24].copy_from_slice(&1u16.to_le_bytes());
    bomb[24..26].copy_from_slice(&24u16.to_le_bytes());
    let input = root.join("resource-bomb.bmp");
    std::fs::write(&input, &bomb).map_err(|e| e.to_string())?;
    if crate::image_import::ImageSource::read(
        &input,
        65536,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .is_ok()
    {
        return Err("Decompression bomb accepted".into());
    }
    if std::fs::read(&path).map_err(|e| e.to_string())? != bytes
        || std::fs::read(&input).map_err(|e| e.to_string())? != bomb
        || d.dirty
        || d.history_counts() != (0, 0)
    {
        return Err("Rejected operation changed input".into());
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    std::fs::remove_file(input).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn resource_edges_and_bomb_preserve_input() {
        let dir = std::env::temp_dir().join(format!("drawer-resource-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let r = super::run(&dir);
        std::fs::remove_dir_all(dir).unwrap();
        r.unwrap();
    }
}
