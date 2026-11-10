use super::*;
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
    paste::ClipboardImage,
};
use std::path::Path;
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 6,
            dpi: 72.5,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].name = "선화 🎨".into();
    d.layers[0].pixels[2 * 8 + 3] = [30, 80, 220, 128];
    d.layers[0].pixels[2 * 8 + 4] = [210, 40, 30, 255];
    d.scene.selection = Some(Rect {
        min: [1., 1.],
        max: [7., 5.],
    });
    Ok(d)
}
pub fn run(root: &Path) -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    let history = d.history_counts();
    let packet = copy(&d, 0, Bounds::Content, B)?;
    let box_packet = copy(&d, 0, Bounds::Selection, B)?;
    if packet.origin != Some([3, 2])
        || packet.image.width != 2
        || packet.image.height != 1
        || box_packet.origin != Some([1, 1])
        || box_packet.image.width != 6
        || box_packet.image.height != 4
    {
        return Err("Content/selection-boundary copy geometry".into());
    }
    let bytes = packet.encode(B)?;
    let read = ClipboardImage::decode(&bytes, B)?;
    if read.name.as_deref() != Some("선화 🎨")
        || read.origin != packet.origin
        || read.image != packet.image
    {
        return Err("Named raster v2 clipboard roundtrip".into());
    }
    let legacy = ClipboardImage {
        name: None,
        color: None,
        image: packet.image.clone(),
        origin: packet.origin,
    };
    let old = legacy.encode(B)?;
    if !old.starts_with(b"DRAWCP\r\n") || ClipboardImage::decode(&old, B)?.name.is_some() {
        return Err("Raster clipboard v1 compatibility".into());
    }
    let mut nested = bytes.clone();
    let name_len = u32::from_le_bytes(nested[8..12].try_into().unwrap()) as usize;
    nested[12 + name_len..20 + name_len].copy_from_slice(b"DRAWCP2\n");
    if ClipboardImage::decode(&nested, B).is_ok() {
        return Err("Nested clipboard envelope accepted".into());
    }
    let mut corrupt = old.clone();
    corrupt[16..20].copy_from_slice(&99u32.to_le_bytes());
    if ClipboardImage::decode(&corrupt, B).is_ok() {
        return Err("Unknown clipboard version/tag accepted".into());
    }
    let rgba = public_rgba(&packet, B)?;
    let mut png = vec![];
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, 2, 1, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
    }
    if decode_png(&png, B)?.image != packet.image || decode_png(&png, 16).is_ok() {
        return Err("Public PNG alpha or memory limit".into());
    }
    let bmp = crate::raster_channels::bmp(
        &rgba,
        [2, 1],
        true,
        96.,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    if decode_dibv5(&bmp[14..], B)?.image != packet.image {
        return Err("Public DIBV5 alpha readback".into());
    }
    let rgba2 = [255, 0, 0, 128, 0, 200, 0, 64, 0, 0, 0, 0, 10, 20, 220, 255];
    let mut png2 = vec![];
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png2)
            .write_image(&rgba2, 2, 2, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
    }
    let bmp2 = crate::raster_channels::bmp(
        &rgba2,
        [2, 2],
        true,
        96.,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    for packet in [decode_png(&png2, B)?, decode_dibv5(&bmp2[14..], B)?] {
        if packet.image.width != 2
            || packet.image.height != 2
            || packet
                .image
                .pixels
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
                != rgba2
        {
            return Err("Public2x2 alpha/row orientation mismatch".into());
        }
    }
    let mut remote = bmp[14..].to_vec();
    remote[56..60].copy_from_slice(&0x4c494e4bu32.to_le_bytes());
    if decode_dibv5(&remote, B).is_ok() || decode_dibv5(&bmp[14..80], B).is_ok() {
        return Err("Unsupported/truncated DIBV5 profile accepted".into());
    }
    if d.snapshot() != before || d.history_counts() != history {
        return Err("Copy changed source/history".into());
    }
    let state = crate::paste::prepare_packet(&d, &read, read.origin.unwrap(), B)?;
    if state.layers.last().unwrap().name != "선화 🎨"
        || state.layers.last().unwrap().placement.unwrap().offset != [3, 2]
    {
        return Err("Named original-position paste".into());
    }
    d.commit_state(state)?;
    let pasted = d.snapshot();
    if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != pasted || !d.undo() {
        return Err("Raster paste one Undo/Redo".into());
    }
    let state = cut(&d, 0, B)?;
    d.commit_state(state)?;
    if d.layers[0].pixels.iter().any(|p| p[3] > 0) || !d.undo() || d.snapshot() != before {
        return Err("Raster cut content/one Undo".into());
    }
    let mut locked = d.clone();
    locked.layers[0].locked = true;
    if cut(&locked, 0, B).is_ok() {
        return Err("Locked source cut accepted".into());
    }
    let mut empty = d.clone();
    empty.scene.selection_mask = Some(Default::default());
    if copy(&empty, 0, Bounds::Content, B).is_ok() {
        return Err("Empty selection copied".into());
    }
    let history_after_edits = d.history_counts();
    for (ppp, zoom, angle, flipped) in [
        (1., 1., 0., false),
        (1.5, 2.5, 73., true),
        (2., 0.5, -90., false),
    ] {
        d.view.zoom = zoom;
        d.view.angle = angle;
        d.view.flipped = flipped;
        let desired = [-14.5, 80.25];
        d.view
            .center_on(desired, [d.width as f64, d.height as f64], [0.; 2], ppp);
        let got = view_center(&d, ppp)?;
        if (0..2).any(|i| (got[i] - desired[i]).abs() > 1e-9) {
            return Err("Zoom/rotation/flip/DPI view-center geometry".into());
        }
    }
    if d.snapshot() != before || d.history_counts() != history_after_edits {
        return Err("Copy/view geometry changed document data/history".into());
    }
    // Named editable vector groups preserve complete pressure and color data across DPI.
    let mut vectors = crate::vector_scope_verify::fixture()?;
    vectors.scene.lines.layers[0].name = "선 🎨".into();
    vectors.scene.lines.layers[1].name = "곡선".into();
    let scope = vectors
        .scene
        .lines
        .layers
        .iter()
        .map(|l| crate::vector_scope::Picked {
            layer: l.id,
            selection: crate::vector_edit::Selection {
                controls: vec![],
                paths: l.paths.iter().map(|p| p.id).collect(),
            },
        })
        .collect::<Vec<_>>();
    let source = vectors.snapshot();
    let packet = crate::vector_clipboard::copy(&vectors, &scope, false)?;
    let bytes = packet.encode()?;
    let restored = crate::vector_clipboard::Packet::decode(&bytes)?;
    if restored != packet || packet.groups.len() != 2 {
        return Err("Named vector v2 roundtrip".into());
    }
    let mut target = fixture()?;
    target.dpi = 600.;
    target.scene.selection = None;
    let target_before = target.snapshot();
    let (state, selected) = crate::vector_clipboard::paste_new(&target, &restored, [12.5, -7.25])?;
    target.commit_state(state)?;
    if target
        .scene
        .lines
        .layers
        .iter()
        .map(|l| l.name.as_str())
        .collect::<Vec<_>>()
        != vec!["선 🎨", "곡선"]
        || selected.len() != 2
    {
        return Err("Vector layer names/order not retained".into());
    }
    let source_ids = packet
        .paths
        .iter()
        .map(|p| p.id)
        .collect::<std::collections::BTreeSet<_>>();
    // IDs may numerically coincide in an empty different document, but are fresh in the destination.
    let original = target.scene.lines.layers[0].paths[0].clone();
    let (second, _) = crate::vector_clipboard::paste_new(&target, &restored, [0.; 2])?;
    let ids = second
        .scene
        .lines
        .layers
        .iter()
        .flat_map(|l| l.paths.iter().map(|p| p.id))
        .collect::<Vec<_>>();
    if ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
        || original.pressure != packet.paths[0].pressure
        || original.color != packet.paths[0].color
        || source_ids.is_empty()
    {
        return Err("Fresh vector IDs/pressure/color".into());
    }
    let native = root.join("clipboard-editable.drawer");
    crate::files::save(&mut target, &native).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&native, B).map_err(|e| e.to_string())?;
    if reopened.snapshot() != target.snapshot()
        || !target.undo()
        || target.snapshot() != target_before
        || vectors.snapshot() != source
    {
        return Err("Clipboard native editability/source/Undo".into());
    }
    std::fs::remove_file(native).map_err(|e| e.to_string())?;
    let mut legacy = serde_json::to_value(&packet).map_err(|e| e.to_string())?;
    legacy.as_object_mut().unwrap().remove("groups");
    let mut old = b"DRAWVEC1\n".to_vec();
    old.extend(serde_json::to_vec(&legacy).map_err(|e| e.to_string())?);
    if !crate::vector_clipboard::Packet::decode(&old)?
        .groups
        .is_empty()
    {
        return Err("Vector v1 compatibility".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn clipboard_named_versions_bounds_public_alpha_pressure_and_native_undo() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
