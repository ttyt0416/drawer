//! Checked admission costs; estimates never allocate a raster or create an output.
use crate::document::MAX_DIMENSION;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub image: u64,
    pub working: u64,
    pub disk: u64,
}
pub fn bytes(width: u64, height: u64, pixel: u64, layers: u64) -> Result<u64, String> {
    if width == 0 || height == 0 || pixel == 0 || layers == 0 {
        return Err("Resource dimensions/channels/layers must be positive".into());
    }
    width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(pixel))
        .and_then(|n| n.checked_mul(layers))
        .ok_or_else(|| "Resource cost overflow; operation refused before allocation".into())
}
pub fn dense(
    width: i64,
    height: i64,
    pixel: u64,
    layers: u64,
    copies: u64,
) -> Result<Cost, String> {
    if !(1..=i64::from(MAX_DIMENSION)).contains(&width)
        || !(1..=i64::from(MAX_DIMENSION)).contains(&height)
    {
        return Err(format!(
            "Dimensions must be1..{MAX_DIMENSION}; requested {width}x{height}"
        ));
    }
    if !(1..=256).contains(&layers) || copies == 0 {
        return Err("Layer/copy count exceeds the supported range".into());
    }
    let image = bytes(width as u64, height as u64, pixel, layers)?;
    let working = image
        .checked_mul(copies)
        .ok_or("Working memory estimate overflow")?;
    let disk = image
        .checked_add(5 * 1024 * 1024)
        .and_then(|n| n.checked_mul(2))
        .ok_or("Disk estimate overflow")?;
    Ok(Cost {
        image,
        working,
        disk,
    })
}
impl Cost {
    pub fn admit_memory(&self, available: u64) -> Result<(), String> {
        if self.working > available {
            Err(format!(
                "This operation needs {} bytes of working memory; budget is {available}. Current document/files are unchanged",
                self.working
            ))
        } else {
            Ok(())
        }
    }
    pub fn admit_disk(&self, available: u64) -> Result<(), String> {
        if self.disk > available {
            Err(format!(
                "This operation needs {} bytes of additional free disk space; available {available}. Previous files are preserved",
                self.disk
            ))
        } else {
            Ok(())
        }
    }
}
pub fn native_space(document_bytes: u64, previous_bytes: u64) -> Result<Cost, String> {
    if document_bytes > 2 * 1024 * 1024 * 1024 || previous_bytes > 2 * 1024 * 1024 * 1024 {
        return Err("Native save block limit2GiB; choose a supported document/output".into());
    }
    Ok(Cost {
        image: document_bytes,
        working: 0,
        disk: document_bytes
            .checked_add(previous_bytes)
            .and_then(|n| n.checked_add(65536))
            .ok_or("Atomic save space overflow")?,
    })
}
