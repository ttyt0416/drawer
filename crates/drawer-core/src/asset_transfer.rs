use super::*;
use crate::{
    document::{DEFAULT_DOCUMENT_BUDGET as B, Document},
    layer_properties::Target,
    layer_search::Key,
    material_layer::{Image, Layer, Source},
    scene::{Affine, PageObject, Rect},
    sizing::DocumentState,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReplaceSize {
    #[default]
    KeepPixels,
    FitExisting,
}
pub struct Prepared {
    pub state: DocumentState,
    pub object: u64,
    pub notes: Vec<String>,
    document: u64,
    revision: u64,
    frame: u32,
}
impl Prepared {
    pub fn check(&self, d: &Document) -> Result<(), String> {
        if (d.id, d.revision, d.current_frame) != (self.document, self.revision, self.frame) {
            Err("Document/frame changed; prepare the asset again".into())
        } else {
            Ok(())
        }
    }
}
fn snapshot(layer: &Layer) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(layer).map_err(|e| e.to_string())?);
    if let Some(image) = layer.source.image() {
        for p in image.pixels.iter() {
            digest.update(p);
        }
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    index: &Index,
    path: &Path,
    d: &Document,
    target: Option<u64>,
    size: ReplaceSize,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    let entry = index.get(path)?;
    if entry.kind != Kind::Image {
        return Err("Insert/replace requires an image asset; use the matching preset exchange editor for other types".into());
    }
    let (absolute, bytes) = index.contents(path, cancel)?;
    let source =
        crate::image_import::ImageSource::from_encoded(absolute, bytes, budget.min(B / 4), cancel)
            .map_err(|e| e.to_string())?;
    let mut notes = vec![format!(
        "Pinned source SHA256 {}. Image objects embed an sRGB RGBA8 copy; source file changes never update it automatically.",
        entry.sha256
    )];
    if source.pages.len() > 1 {
        notes.push(format!(
            "First TIFF page of{}; use Image Import for another page",
            source.pages.len()
        ));
    }
    let page = source
        .decode(0, true, budget.min(B / 4), cancel)
        .map_err(|e| e.to_string())?;
    if page.bits > 8 {
        notes.push("16-bit source is quantized to the image object's RGBA8 snapshot. Use Image as new layer for a precision-preserving raster import.".into());
    }
    notes.extend(page.warnings);
    if page.image.pixels.len() > 4 * 1024 * 1024
        || page
            .image
            .pixels
            .len()
            .saturating_mul(40)
            .saturating_add(d.resident_bytes())
            > budget
    {
        return Err("Image asset preview exceeds work/memory limit".into());
    }
    let image = Image {
        width: page.image.width,
        height: page.image.height,
        pixels: Arc::new(page.image.pixels),
    };
    let layer = if let Some(id) = target {
        let n = d
            .scene
            .catalog
            .objects
            .index(id, d.scene.objects.len())
            .ok_or("Select an image object to replace")?;
        let PageObject::Material(old) = &d.scene.objects[n] else {
            return Err("Select a material image object to replace".into());
        };
        let Source::Image(old_image) = &old.source else {
            return Err("Replacement requires an existing image source".into());
        };
        let mut layer = old.as_ref().clone();
        let image = if size == ReplaceSize::FitExisting {
            let raw = image.pixels.iter().flat_map(|p| *p).collect::<Vec<_>>();
            let raw = image::RgbaImage::from_raw(image.width, image.height, raw)
                .ok_or("Invalid image pixels")?;
            let resized = image::imageops::resize(
                &raw,
                old_image.width,
                old_image.height,
                image::imageops::FilterType::Nearest,
            );
            notes.push(format!("Fit nearest: new{}×{} source resampled to{}×{}; position, affine transform, crop, tiling and warp retained",image.width,image.height,old_image.width,old_image.height));
            Image {
                width: old_image.width,
                height: old_image.height,
                pixels: Arc::new(resized.into_raw().as_chunks::<4>().0.to_vec()),
            }
        } else {
            layer.area.max = [
                layer.area.min[0] + image.width as f64,
                layer.area.min[1] + image.height as f64,
            ];
            notes.push("Keep pixel size: position/affine transform, local crop origin, tiling and normalized warp retained; extent follows new pixel dimensions".into());
            image
        };
        layer.source = Source::Image(image);
        layer
    } else {
        Layer {
            area: Rect {
                min: [0.; 2],
                max: [image.width as f64, image.height as f64],
            },
            source: Source::Image(image),
            transform: Affine([
                1.,
                0.,
                0.,
                1.,
                (d.width as f64 - page.image.width as f64) / 2.,
                (d.height as f64 - page.image.height as f64) / 2.,
            ])
            .then(d.scene.transform.inverse()),
            repeat: [false; 2],
            mapping: Default::default(),
        }
    };
    canceled(cancel)?;
    let snapshot_hash = snapshot(&layer)?;
    let (mut state, object) =
        crate::material_layer::prepare(d, target, &entry.metadata.name, layer)?;
    state
        .scene
        .catalog
        .object_info
        .entry(object)
        .or_default()
        .asset_version = Some(Version {
        source_hash: entry.sha256.clone(),
        snapshot_hash,
        name: entry.metadata.name.clone(),
    });
    if !state.scene.valid() {
        return Err("Asset snapshot produced invalid document metadata".into());
    }
    canceled(cancel)?;
    notes.push(
        "Preview only. Apply is one document Undo; library edits/Undo are independent.".into(),
    );
    Ok(Prepared {
        state,
        object,
        notes,
        document: d.id,
        revision: d.revision,
        frame: d.current_frame,
    })
}
pub fn reference_count(documents: &[Document], root: &Path, entry: &Entry) -> usize {
    let path = root.join(&entry.path);
    documents
        .iter()
        .map(|d| {
            let mut n = usize::from(
                d.path.as_ref() == Some(&path) || d.source_path.as_ref() == Some(&path),
            );
            n += d
                .scene
                .catalog
                .object_info
                .values()
                .chain(d.scene.catalog.rasters.values())
                .chain(d.scene.catalog.vectors.values())
                .filter(|i| {
                    i.asset_version
                        .as_ref()
                        .is_some_and(|v| v.source_hash == entry.sha256)
                })
                .count();
            n += d
                .scene
                .file_objects
                .entries
                .iter()
                .filter(|l| l.path == path)
                .count();
            n
        })
        .sum()
}
pub fn register(
    root: &Path,
    d: &Document,
    key: Key,
    name: &str,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    if !(Metadata {
        name: name.into(),
        ..Default::default()
    })
    .valid()
    {
        return Err("Choose a material name1..64 characters without controls".into());
    }
    canceled(cancel)?;
    let packet = if let Some(Target::Raster(id)) = crate::layer_catalog::target(d, key) {
        crate::selection_transfer::copy(
            d,
            crate::layers::index(d, id).ok_or("Missing raster")?,
            B / 4,
        )?
    } else {
        let converted = crate::raster_conversion::prepare(
            d,
            key,
            crate::raster_conversion::Action::Rasterize(d.scene.catalog.expression),
        )?;
        let copy = crate::layer_merge::preview(&converted.state, d.current_frame)?;
        let Some(Target::Raster(id)) = crate::layer_catalog::target(&copy, converted.selected)
        else {
            return Err("Raster snapshot of this source is unavailable".into());
        };
        crate::selection_transfer::copy(
            &copy,
            crate::layers::index(&copy, id).ok_or("Missing captured raster")?,
            B / 4,
        )?
    };
    let mut encoded = packet.encode(B / 4)?;
    encoded.extend(d.dpi.to_le_bytes());
    let identity = hash(&encoded);
    let bits = if packet.color.is_some() { 16 } else { 8 };
    let imported = crate::image_import::ImportedPage {
        image: packet.image,
        color: packet.color,
        orientation: image::metadata::Orientation::NoTransforms,
        profile_applied: false,
        bits,
        warnings: vec![],
    };
    let mut copy = imported.document(B / 4).map_err(|e| e.to_string())?;
    copy.dpi = d.dpi;
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    if !crate::font_library::local(&canonical)
        || linked(&fs::symlink_metadata(root).map_err(|e| e.to_string())?)
    {
        return Err("Choose a local material folder without links".into());
    }
    let registered = canonical.join("Registered");
    if !registered.exists() {
        fs::create_dir(&registered).map_err(|e| e.to_string())?;
    }
    let rel = PathBuf::from("Registered").join(format!("{identity}.png"));
    let target = checked(&canonical, &rel, false)?;
    let temp_rel = PathBuf::from("Registered").join(format!("{identity}.register-write.png"));
    let temp = checked(&canonical, &temp_rel, false)?;
    if fs::symlink_metadata(&temp).is_ok() {
        return Err("A foreign registration temporary file exists; it was preserved".into());
    }
    let result = (|| {
        crate::png_export::export_with(
            &copy,
            &temp,
            B / 2,
            crate::png_export::Options {
                depth: if bits == 16 {
                    crate::color_precision::Depth::Sixteen
                } else {
                    crate::color_precision::Depth::Eight
                },
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        canceled(cancel)?;
        if target.exists() {
            if bytes(&target, FILE_LIMIT)? != bytes(&temp, FILE_LIMIT)? {
                return Err(
                    "Registered filename exists with different contents; original preserved".into(),
                );
            }
        } else {
            fs::hard_link(&temp, &target).map_err(|e| e.to_string())?;
        }
        Ok(rel)
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}
