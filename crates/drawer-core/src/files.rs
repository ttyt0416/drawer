use crate::document::{Background, CanvasSpec, Document, MAX_DIMENSION, PlacedRaster, RasterLayer};
use std::{
    fs,
    io::{self, BufReader, BufWriter, Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"DRAWER\r\n";
pub(crate) const VERSION: u32 = 101;
const MAX_SCENE_BYTES: usize = 4 * 1024 * 1024;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

pub fn open(path: &Path, budget: usize) -> io::Result<Document> {
    let path = path.canonicalize()?;
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    match ext.as_str() {
        "drawer" => read_native(&path, budget),
        "png" => read_png(&path, budget),
        "psd" => crate::psd::open(&path, budget),
        _ => Err(invalid(
            "Supported open formats: .drawer, .png, raw RGB8 .psd merged preview",
        )),
    }
}
fn read_png(path: &Path, budget: usize) -> io::Result<Document> {
    let mut reader = image::ImageReader::open(path)?;
    reader.set_format(image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(budget as u64);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| invalid(error.to_string()))?
        .into_rgba8();
    let mut document = Document::create(
        CanvasSpec {
            width: image.width(),
            height: image.height(),
            dpi: 300.0,
            background: Background::Transparent,
        },
        budget,
    )
    .map_err(|e| invalid(e.to_string()))?;
    for (destination, source) in document.layers[0].pixels.iter_mut().zip(image.pixels()) {
        *destination = source.0;
    }
    document.title = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    document.source_path = Some(path.into());
    Ok(document)
}
fn read_native(path: &Path, budget: usize) -> io::Result<Document> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > budget as u64 + MAX_SCENE_BYTES as u64 + 1024 * 1024 {
        return Err(invalid("Document exceeds memory budget"));
    }
    read_native_reader(path, BufReader::new(file), budget)
}
pub(crate) fn read_native_snapshot(
    path: &Path,
    bytes: &[u8],
    budget: usize,
) -> io::Result<Document> {
    if bytes.len()
        > budget
            .saturating_add(MAX_SCENE_BYTES)
            .saturating_add(1024 * 1024)
    {
        return Err(invalid("Document exceeds memory budget"));
    }
    read_native_reader(path, std::io::Cursor::new(bytes), budget)
}
fn read_native_reader(path: &Path, mut reader: impl Read, budget: usize) -> io::Result<Document> {
    let mut magic = [0; 8];
    reader.read_exact(&mut magic)?;
    let version = read_u32(&mut reader)?;
    if &magic != MAGIC || !(1..=VERSION).contains(&version) {
        return Err(invalid("Unsupported Drawer document version"));
    }
    let width = read_u32(&mut reader)?;
    let height = read_u32(&mut reader)?;
    let dpi = f64::from_bits(read_u64(&mut reader)?);
    let layers = read_u32(&mut reader)? as usize;
    if !(1..=256).contains(&layers) {
        return Err(invalid("Invalid layer count"));
    }
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err(invalid("Invalid document dimensions"));
    }
    let mut document = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi,
            background: Background::Transparent,
        },
        budget,
    )
    .map_err(|e| invalid(e.to_string()))?;
    document.width = width;
    document.height = height;
    document.layers.clear();
    let mut used = 0usize;
    for _ in 0..layers {
        let name_len = read_u32(&mut reader)? as usize;
        if !(1..=4096).contains(&name_len) {
            return Err(invalid("Invalid layer name"));
        }
        let mut name = vec![0; name_len];
        reader.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| invalid("Layer name must be UTF-8"))?;
        let placement = if version >= 4 {
            match read_u32(&mut reader)? {
                0 => None,
                1 => Some(PlacedRaster {
                    width: read_u32(&mut reader)?,
                    height: read_u32(&mut reader)?,
                    offset: [read_u64(&mut reader)? as i64, read_u64(&mut reader)? as i64],
                }),
                _ => return Err(invalid("Invalid raster placement tag")),
            }
        } else {
            None
        };
        let locked = if version >= 7 {
            match read_u32(&mut reader)? {
                0 => false,
                1 => true,
                _ => return Err(invalid("Invalid layer lock tag")),
            }
        } else {
            false
        };
        let extent = placement.unwrap_or(PlacedRaster {
            width,
            height,
            offset: [0; 2],
        });
        if !extent.valid() {
            return Err(invalid("Invalid raster placement"));
        }
        let count = extent.width as usize * extent.height as usize;
        used = used
            .checked_add(count * 4)
            .ok_or_else(|| invalid("Document size overflow"))?;
        if used > budget || read_u64(&mut reader)? != count as u64 * 4 {
            return Err(invalid("Invalid or oversized raster payload"));
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| invalid("Not enough document memory"))?;
        pixels.resize(count, [0; 4]);
        for chunk in pixels.chunks_mut(4096) {
            let mut buffer = [0u8; 16384];
            let count = chunk.len() * 4;
            reader.read_exact(&mut buffer[..count])?;
            for (pixel, bytes) in chunk.iter_mut().zip(buffer[..count].as_chunks::<4>().0) {
                pixel.copy_from_slice(bytes);
            }
        }
        document.layers.push(RasterLayer {
            precise: None,
            locked,
            name,
            pixels,
            placement,
        });
    }
    if version >= 2 {
        let length = read_u32(&mut reader)? as usize;
        if length > MAX_SCENE_BYTES {
            return Err(invalid("Page geometry is too large"));
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes)?;
        document.scene = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
        if version < 82 && crate::extended_filters::active(&document.scene) {
            return Err(invalid("Finishing filter recipes require native version82"));
        }
        if version < 81
            && (document.scene.mesh_edit.is_some()
                || document
                    .scene
                    .surface_effects
                    .rasters
                    .values()
                    .chain(document.scene.surface_effects.baked.values())
                    .any(|s| !s.space.is_identity())
                || document.scene.layer_groups.transforms.iter().any(|f| {
                    !matches!(
                        f.key,
                        crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(_))
                    )
                }))
        {
            return Err(invalid(
                "Editable mesh sources and transform frames require native version81",
            ));
        }
        if version < 101
            && (document.scene.keyframes.camera_output || document.scene.camera_guides.is_some())
        {
            return Err(invalid("Final camera and path guides require native101"));
        }
        document.camera_guides = document.scene.camera_guides.take().unwrap_or_default();
        if !document.camera_guides.valid() {
            return Err(invalid("Invalid camera guide preferences"));
        }
        if version < 100 && document.scene.light_table.is_some() {
            return Err(invalid("Light-table objects require native100"));
        }
        let auxiliary = document.scene.light_table.take().unwrap_or_default();
        if !auxiliary.valid(true) {
            return Err(invalid("Invalid light-table metadata"));
        }
        document.light_table = crate::light_table::Session::loaded(auxiliary);
        if version < 99 && document.scene.onion.is_some() {
            return Err(invalid("Onion preferences require native99"));
        }
        document.onion = document
            .scene
            .onion
            .take()
            .unwrap_or_default()
            .preferences();
        if !document.onion.valid() {
            return Err(invalid("Invalid onion preferences"));
        }
        if version < 98 && !document.scene.keyframes.is_default() {
            return Err(invalid("Layer animation requires native98"));
        }
        if version < 97 && !document.scene.timing.is_default() {
            return Err(invalid("Timeline timing requires native97"));
        }
        if version < 96 && !document.scene.cels.is_default() {
            return Err(invalid("Editable cels require native96"));
        }
        if version < 95 && !document.scene.shots.is_default() {
            return Err(invalid("Shot metadata requires native95"));
        }
        if version < 94
            && (!document.scene.page_parts.is_default()
                || crate::frame_ops::requires94(&document.scene))
        {
            return Err(invalid(
                "Editable page containers and compound/clipped frames require version94",
            ));
        }
        if version < 93
            && (!document.scene.comics.is_default()
                || document
                    .scene
                    .objects
                    .iter()
                    .any(|o| matches!(o, crate::scene::PageObject::Comic(_))))
        {
            return Err(invalid("Comic objects require native version93"));
        }
        if version<92 && document.scene.objects.iter().any(|o|matches!(o,crate::scene::PageObject::Text{layout:Some(s),..} if s.circle.is_some()||s.effects.is_some())){return Err(invalid("Circular text and text effects require native version92"));}
        if version<91 && document.scene.objects.iter().any(|o|matches!(o,crate::scene::PageObject::Text{layout:Some(s),..} if s.typeset.is_some())) {return Err(invalid("Advanced typography requires native version91"));}
        if version<90 && document.scene.objects.iter().any(|o|matches!(o,crate::scene::PageObject::Text{layout:Some(s),..} if s.generation!=1 || !s.frame.is_identity())) {return Err(invalid("Text layout generations and transforms require native version90"));}
        if version<89 && document.scene.objects.iter().any(|o|matches!(o,crate::scene::PageObject::Text{layout:Some(s),..} if s.fonts().iter().any(|f|matches!(f,crate::text_layout::Font::Local(_))))) {return Err(invalid("Local font references require native version89"));}
        if version<88 && document.scene.objects.iter().any(|o|matches!(o,crate::scene::PageObject::Text{layout:Some(s),..} if s.rich.is_some())) {return Err(invalid("Grapheme formatting requires native version88"));}
        if version < 87
            && document.scene.objects.iter().any(
                |o| matches!(o,crate::scene::PageObject::Text{layout:Some(s),..} if s.extended()),
            )
        {
            return Err(invalid("Extended text layout requires native version87"));
        }
        if version < 86
            && document.scene.objects.iter().any(|o| {
                matches!(
                    o,
                    crate::scene::PageObject::Text {
                        layout: Some(_),
                        ..
                    }
                )
            })
        {
            return Err(invalid("Text layout requires native version86"));
        }
        if version < 85 && document.scene.last_quick_filter.is_some() {
            return Err(invalid("Filter routing requires native version85"));
        }
        if version < 84 && crate::cleanup_filters::active(&document.scene) {
            return Err(invalid("Dust/tone tools require native version84"));
        }
        if version < 83 && crate::filter_options::active(&document.scene) {
            return Err(invalid("Filter options require native version83"));
        }
        if version < 80 && document.scene.last_transform.is_some() {
            return Err(invalid("Transform receipts require native version80"));
        }
        if version < 79 && (document.scene.objects.iter().any(|o| matches!(o, crate::scene::PageObject::Material(m) if matches!(m.source, crate::material_layer::Source::Metal(_))))
            || document.scene.file_objects.entries.iter().any(|l| matches!(l.cache.source, crate::material_layer::Source::Metal(_)))) {
            return Err(invalid("Metallic materials require native version79"));
        }
        if version < 78
            && (document.scene.last_color_deepen.is_some()
                || document.scene.layer_effects.rasters.values().any(|e| {
                    e.adjustment.is_some_and(|a| {
                        matches!(a.local, Some(crate::local_color::Correction::Deepen(_)))
                    })
                }))
        {
            return Err(invalid("Color-deepen settings require native version78"));
        }
        if version < 77 && crate::local_color::active(&document.scene) {
            return Err(invalid("Local color correction requires native version77"));
        }
        if version < 76 && document.scene.last_batch_filter.is_some() {
            return Err(invalid("Batch filter records require native version76"));
        }
        if version < 75
            && document.scene.last_detail_filter.is_some_and(|s| {
                matches!(
                    s.operation,
                    crate::detail_filter::Operation::Pixels(crate::pixel_effects::Operation::Warp(
                        _
                    ))
                )
            })
        {
            return Err(invalid("Distortion recipes require native version75"));
        }
        if version < 74 && !document.scene.surface_effects.is_default() {
            return Err(invalid("Surface effects require native version 74"));
        }
        if version < 73
            && document.scene.last_detail_filter.is_some_and(|s| {
                matches!(
                    s.operation,
                    crate::detail_filter::Operation::Pixels(
                        crate::pixel_effects::Operation::Style(_)
                    )
                )
            })
        {
            return Err(invalid(
                "Stylized effect recipes require document version73",
            ));
        }
        if version < 72
            && document
                .scene
                .last_detail_filter
                .is_some_and(|s| matches!(s.operation, crate::detail_filter::Operation::Pixels(_)))
        {
            return Err(invalid("Pixel effect recipes require document version72"));
        }
        if version < 71 && document.scene.last_detail_filter.is_some() {
            return Err(invalid(
                "Baked detail filter settings require document version71",
            ));
        }
        if version < 70 && document.scene.last_spatial_blur.is_some() {
            return Err(invalid(
                "Baked spatial blur settings require document version70",
            ));
        }
        if version < 69 && crate::layer_effects::requires_v69(&document.scene) {
            return Err(invalid(
                "RGB inversion correction requires document version69",
            ));
        }
        if version < 68 && crate::svg_style::requires_v68(&document.scene) {
            return Err(invalid("SVG vector paints require document version 68"));
        }
        if version < 67 && crate::vector_tip::requires_v67(&document.scene) {
            return Err(invalid("Vector tip styles require document version 67"));
        }
        if version < 66 && crate::vector_nodes::requires_v66(&document.scene) {
            return Err(invalid("Cubic nodes require document version 66"));
        }
        if version < 65 && crate::vector_pressure::requires_v65(&document.scene) {
            return Err(invalid(
                "Fragment pressure positions require document version 65",
            ));
        }
        if version < 64 && crate::vector_pressure::requires_v64(&document.scene) {
            return Err(invalid(
                "Vector pressure and zero stroke width require document version 64",
            ));
        }
        if version < 63
            && (crate::file_sources::requires_v63(&document.scene)
                || crate::material_mapping::requires_v63(&document.scene))
        {
            return Err(invalid(
                "Source selection and material deformation require document version 63",
            ));
        }
        if version < 61 && document.scene.paper_background.is_some() {
            return Err(invalid(
                "Paper background identity requires document version 61",
            ));
        }
        if version < 60 && crate::expression_color::requires_v60(&document.scene) {
            return Err(invalid("Expression thresholds require document version 60"));
        }
        if version < 59 && crate::layer_groups::requires_v59(&document.scene) {
            return Err(invalid(
                "Linked groups and local appearance frames require document version 59",
            ));
        }
        if version < 58 && !document.scene.layer_comps.is_default() {
            return Err(invalid("Layer comps require document version 58"));
        }
        if version < 56 && !document.scene.render_order.is_default() {
            return Err(invalid(
                "Source raster positions require document version 56",
            ));
        }
        if version < 55 && !document.scene.file_objects.is_default() {
            return Err(invalid("File objects require document version 55"));
        }
        if !(if version >= 54 {
            document.scene.metadata_valid()
        } else {
            document.scene.valid()
        }) || !document.scene.layer_ids.matches(document.layers.len())
            || !document.scene.paper.valid_for(document.layers.len())
            || document.memory_bytes() > budget
        {
            return Err(invalid("Invalid or oversized page geometry"));
        }
    }
    if version >= 6 {
        match read_u32(&mut reader)? {
            0 => {}
            1 => {
                document.animation = Some(crate::animation::Timeline::read(
                    &mut reader,
                    budget.saturating_sub(document.memory_bytes()),
                )?)
            }
            _ => return Err(invalid("Invalid animation tag")),
        }
        if document.memory_bytes() > budget {
            return Err(invalid("Animation exceeds document memory budget"));
        }
    }
    if version >= 54 {
        crate::material_layer::read(&mut document.scene, &mut reader)?;
    }
    if version >= 81 {
        crate::mesh_edit::read(&mut document.scene, &mut reader)?;
    }
    if !(if version >= 94 {
        document.scene.metadata_valid()
    } else {
        document.scene.valid()
    }) || document.memory_bytes() > budget
    {
        return Err(invalid("Invalid material image payload or budget"));
    }
    if !document
        .scene
        .catalog
        .valid_animation(document.animation.as_ref())
    {
        return Err(invalid("Invalid animation source identities"));
    }
    if !crate::paper_background::valid_live(&document)
        || !crate::layer_groups::valid_live(&document)
        || !crate::render_order::valid_live(&document)
        || !crate::file_objects::valid_live(&document)
        || !crate::surface_effects::valid_live(&document)
    {
        return Err(invalid("Invalid file-layer source identity"));
    }
    crate::appearance::limits(
        document.width,
        document.height,
        &document.layers,
        &document.scene,
    )
    .map_err(invalid)?;
    if version >= 62 {
        crate::color_storage::read(&mut document, &mut reader, budget)?;
    }
    if version >= 94 {
        crate::page_parts::read(&mut document, &mut reader, budget)?;
    }
    if version >= 96 {
        crate::cels::read(&mut document, &mut reader, budget)?;
    }
    if version >= 97 {
        crate::timeline_timing::read(&mut document, &mut reader, budget)?;
    }
    if version >= 100 {
        crate::light_table::read(&mut document, &mut reader, budget)?;
    }
    if !document.scene.valid()
        || !crate::keyframes::valid_document(&document)
        || !crate::timeline_timing::valid_document(&document)
        || !crate::cels::valid_document(&document)
        || !crate::shot_exchange::valid_document(&document)
        || document.memory_bytes() > budget
    {
        return Err(invalid("Invalid embedded page sources or memory budget"));
    }
    let mut trailing = [0];
    if reader.read(&mut trailing)? != 0 {
        return Err(invalid("Unexpected document data"));
    }
    crate::text_layout::validate_scene(&document.scene).map_err(invalid)?;
    crate::light_table::save_destination(&document, path).map_err(invalid)?;
    document.mark_saved(path.into());
    Ok(document)
}

pub fn save(document: &mut Document, path: &Path) -> io::Result<()> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("drawer"))
    {
        return Err(invalid("Editable documents must be saved as .drawer"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = parent.canonicalize()?.join(
        path.file_name()
            .ok_or_else(|| invalid("Missing filename"))?,
    );
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(invalid("Choose a regular destination file"));
    }
    let path = if path.exists() {
        path.canonicalize()?
    } else {
        path
    };
    crate::light_table::save_destination(document, &path).map_err(invalid)?;
    let scene = native_scene(document)?;
    let mut temp_name = path.file_name().unwrap().to_owned();
    temp_name.push(".drawer-write.tmp");
    let temporary = path.with_file_name(temp_name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let mut writer = BufWriter::new(file);
    let result = (|| {
        write_payload(document, &scene, &mut writer)?;
        writer.flush()?;
        writer.get_ref().sync_all()
    })();
    drop(writer);
    let result = result.and_then(|_| fs::rename(&temporary, &path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    document.mark_saved(path);
    Ok(())
}

fn native_scene(document: &Document) -> io::Result<Vec<u8>> {
    if !crate::keyframes::valid_document(document)
        || !crate::timeline_timing::valid_document(document)
        || !crate::timeline_timing::original_hashes(document)
    {
        return Err(invalid("Invalid timeline clip/PCM backing"));
    }
    if !crate::cels::valid_document(document) {
        return Err(invalid("Invalid cel definitions/references"));
    }
    if !crate::shot_exchange::valid_document(document) {
        return Err(invalid("Invalid shot source identities"));
    }
    if !(1..=MAX_DIMENSION).contains(&document.width)
        || !(1..=MAX_DIMENSION).contains(&document.height)
        || !document.dpi.is_finite()
        || document.dpi <= 0.0
        || !(1..=256).contains(&document.layers.len())
    {
        return Err(invalid("Invalid document"));
    }
    for layer in &document.layers {
        if !layer.valid(document.width, document.height) {
            return Err(invalid("Invalid raster layer"));
        }
    }
    if !document.color.valid_layers(&document.layers) {
        return Err(invalid(
            "Invalid document channel precision/profile or stale pixel preview",
        ));
    }
    if document.animation.as_ref().is_some_and(|a| !a.valid()) {
        return Err(invalid("Invalid animation"));
    }
    if !document.scene.valid()
        || !document.scene.layer_ids.matches(document.layers.len())
        || !document.scene.paper.valid_for(document.layers.len())
        || document
            .scene
            .size_bytes()
            .saturating_sub(document.scene.page_parts.payload_bytes())
            .saturating_sub(document.scene.cels.payload_bytes())
            .saturating_sub(document.scene.timing.payload_bytes())
            > MAX_SCENE_BYTES
    {
        return Err(invalid("Invalid or oversized page geometry"));
    }
    if !document
        .scene
        .catalog
        .valid_animation(document.animation.as_ref())
    {
        return Err(invalid("Invalid animation source identities"));
    }
    if !crate::paper_background::valid_live(document)
        || !crate::layer_groups::valid_live(document)
        || !crate::render_order::valid_live(document)
        || !crate::file_objects::valid_live(document)
        || !crate::surface_effects::valid_live(document)
    {
        return Err(invalid("Invalid file-layer source identity"));
    }
    crate::appearance::limits(
        document.width,
        document.height,
        &document.layers,
        &document.scene,
    )
    .map_err(invalid)?;
    crate::text_layout::validate_scene(&document.scene).map_err(invalid)?;
    if !document.camera_guides.valid()
        || document.scene.camera_guides.is_some()
        || !document.onion.valid()
        || document.scene.onion.is_some()
        || document.scene.light_table.is_some()
        || !document.light_table.data.valid(false)
        || !document.light_table.data.hashes()
    {
        return Err(invalid("Invalid view preferences transport"));
    }
    #[derive(serde::Serialize)]
    struct Metadata<'a> {
        #[serde(flatten)]
        scene: &'a crate::scene::Scene,
        #[serde(skip_serializing_if = "Option::is_none")]
        onion: Option<crate::timeline_view::Onion>,
        #[serde(skip_serializing_if = "Option::is_none")]
        camera_guides: Option<crate::camera2d::Guides>,
        #[serde(skip_serializing_if = "Option::is_none")]
        light_table: Option<&'a crate::light_table::Data>,
    }
    let scene = serde_json::to_vec(&Metadata {
        scene: &document.scene,
        camera_guides: (document.camera_guides != Default::default())
            .then_some(document.camera_guides),
        light_table: (!document.light_table.data.is_default())
            .then_some(&document.light_table.data),
        onion: (document.onion.preferences() != crate::timeline_view::Onion::default())
            .then_some(document.onion.preferences()),
    })
    .map_err(|e| invalid(e.to_string()))?;
    if scene.len() > MAX_SCENE_BYTES {
        return Err(invalid("Page geometry is too large"));
    }
    Ok(scene)
}

pub(crate) fn native_bytes(document: &Document) -> io::Result<Vec<u8>> {
    if document.memory_bytes() > 64 * 1024 * 1024 {
        return Err(invalid("Contained native page exceeds64MiB"));
    }
    let scene = native_scene(document)?;
    let mut bytes = Vec::new();
    write_payload(document, &scene, &mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(invalid("Contained native payload exceeds64MiB"));
    }
    Ok(bytes)
}
fn write_payload(document: &Document, scene: &[u8], writer: &mut impl Write) -> io::Result<()> {
    writer.write_all(MAGIC)?;
    writer.write_all(&VERSION.to_le_bytes())?;
    writer.write_all(&document.width.to_le_bytes())?;
    writer.write_all(&document.height.to_le_bytes())?;
    writer.write_all(&document.dpi.to_bits().to_le_bytes())?;
    writer.write_all(&(document.layers.len() as u32).to_le_bytes())?;
    for layer in &document.layers {
        writer.write_all(&(layer.name.len() as u32).to_le_bytes())?;
        writer.write_all(layer.name.as_bytes())?;
        if let Some(p) = layer.placement {
            writer.write_all(&1u32.to_le_bytes())?;
            writer.write_all(&p.width.to_le_bytes())?;
            writer.write_all(&p.height.to_le_bytes())?;
            writer.write_all(&p.offset[0].to_le_bytes())?;
            writer.write_all(&p.offset[1].to_le_bytes())?;
        } else {
            writer.write_all(&0u32.to_le_bytes())?;
        }
        writer.write_all(&u32::from(layer.locked).to_le_bytes())?;
        writer.write_all(&(layer.pixels.len() as u64 * 4).to_le_bytes())?;
        for chunk in layer.pixels.chunks(4096) {
            let mut buffer = [0u8; 16384];
            for (bytes, pixel) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(chunk) {
                bytes.copy_from_slice(pixel);
            }
            writer.write_all(&buffer[..chunk.len() * 4])?;
        }
    }
    writer.write_all(&(scene.len() as u32).to_le_bytes())?;
    writer.write_all(scene)?;
    if let Some(animation) = &document.animation {
        writer.write_all(&1u32.to_le_bytes())?;
        animation.write(writer)?;
    } else {
        writer.write_all(&0u32.to_le_bytes())?;
    }
    crate::material_layer::write(&document.scene, writer)?;
    crate::mesh_edit::write(&document.scene, writer)?;
    crate::color_storage::write(document, writer)?;
    crate::page_parts::write(&document.scene, writer)?;
    crate::cels::write(&document.scene, writer)?;
    crate::timeline_timing::write(&document.scene, writer)?;
    crate::light_table::write(document, writer)?;
    Ok(())
}

pub(crate) fn write_new_native(document: &Document, path: &Path) -> io::Result<()> {
    let scene = native_scene(document)?;
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    let mut writer = BufWriter::new(file);
    let result = write_payload(document, &scene, &mut writer)
        .and_then(|_| writer.flush())
        .and_then(|_| writer.get_ref().sync_all());
    drop(writer);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    fn sample() -> Document {
        Document::create(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 144.0,
                background: Background::Transparent,
            },
            16,
        )
        .unwrap()
    }
    #[test]
    fn pigment_style_roundtrip_and_legacy_version_ten_defaults() {
        let root = TempDir::new();
        let path = root.0.join("pigment.drawer");
        let mut d = sample();
        d.scene.mouse_strokes.push(crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: crate::painting::Style {
                clone_stamp: Default::default(),
                variation: Default::default(),
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Default::default(),
                texture: Default::default(),
                material: crate::brush_material::Settings {
                    mode: crate::brush_material::Mode::Water,
                    supply: 0.2,
                    keep_density: true,
                    alpha_lock: true,
                    ..Default::default()
                },
                shape: Default::default(),
                tip: crate::assets::BrushTip::Pen,
                color: [40, 80, 120, 255],
                opacity: 0.5,
                hardness: 0.3,
                spacing: 0.2,
                smoothing: 0.8,
                pressure: Default::default(),
            },
            count: 1,
            ends: Default::default(),
            native_pressure: true,
        });
        save(&mut d, &path).unwrap();
        assert!(open(&path, 4096).unwrap().snapshot() == d.snapshot());
        let bytes = fs::read(&path).unwrap();
        let offset = 32 + 4 + d.layers[0].name.len() + 4 + 4 + 8 + d.layers[0].pixels.len() * 4;
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length]).unwrap();
        for record in value["mouse_strokes"].as_array_mut().unwrap() {
            record["style"].as_object_mut().unwrap().remove("material");
        }
        let json = serde_json::to_vec(&value).unwrap();
        let mut legacy = bytes[..offset].to_vec();
        legacy.extend_from_slice(&(json.len() as u32).to_le_bytes());
        legacy.extend_from_slice(&json);
        legacy.extend_from_slice(&bytes[offset + 4 + length..]);
        legacy[8..12].copy_from_slice(&10u32.to_le_bytes());
        fs::write(&path, legacy).unwrap();
        let mut expected = d.snapshot();
        expected.scene.mouse_strokes[0].style.material = Default::default();
        assert!(open(&path, 4096).unwrap().snapshot() == expected);
    }
    #[test]
    fn frozen_geometry_native_pressure_roundtrip_and_version_nine_defaults() {
        let root = TempDir::new();
        let path = root.0.join("geometry.drawer");
        let mut d = sample();
        d.scene.mouse_strokes.push(crate::mouse_pressure::Metadata {
            layer: 0,
            settings: Default::default(),
            style: crate::painting::Style {
                clone_stamp: Default::default(),
                variation: Default::default(),
                pattern: Default::default(),
                dynamics: Default::default(),
                sequence: Default::default(),
                filters: Default::default(),
                profile: Default::default(),
                texture: Default::default(),
                material: Default::default(),
                shape: Default::default(),
                tip: crate::assets::BrushTip::Pen,
                color: [40, 80, 120, 255],
                opacity: 0.5,
                hardness: 0.3,
                spacing: 0.2,
                smoothing: 0.8,
                pressure: Default::default(),
            },
            count: 1,
            ends: crate::stroke_ends::Settings {
                method: Default::default(),
                fade_minimum: 0.0,
                start: 2.0,
                end: 3.0,
            },
            native_pressure: true,
        });
        save(&mut d, &path).unwrap();
        assert!(open(&path, 4096).unwrap().snapshot() == d.snapshot());
        let bytes = fs::read(&path).unwrap();
        let offset = 32 + 4 + d.layers[0].name.len() + 4 + 4 + 8 + d.layers[0].pixels.len() * 4;
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length]).unwrap();
        for record in value["mouse_strokes"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("ends");
            record.as_object_mut().unwrap().remove("native_pressure");
        }
        let old_json = serde_json::to_vec(&value).unwrap();
        let mut legacy = bytes[..offset].to_vec();
        legacy.extend_from_slice(&(old_json.len() as u32).to_le_bytes());
        legacy.extend_from_slice(&old_json);
        legacy.extend_from_slice(&bytes[offset + 4 + length..]);
        legacy[8..12].copy_from_slice(&9u32.to_le_bytes());
        fs::write(&path, legacy).unwrap();
        let reopened = open(&path, 4096).unwrap();
        assert_eq!(reopened.layers, d.layers);
        assert_eq!(
            reopened.scene.mouse_strokes[0].style,
            d.scene.mouse_strokes[0].style
        );
        assert_eq!(reopened.scene.mouse_strokes[0].ends, Default::default());
        assert!(!reopened.scene.mouse_strokes[0].native_pressure);
    }
    #[test]
    fn native_round_trip_preserves_layers_and_failed_save_keeps_previous_file() {
        let root = TempDir::new();
        let path = root.0.join("same.drawer");
        let mut document = sample();
        document.layers[0].name = "원본 레이어".into();
        document.edit_pixel(0, 1, 0, [20, 40, 60, 255]);
        document.layers.push(RasterLayer {
            precise: None,
            locked: false,
            name: "Second".into(),
            pixels: vec![[0; 4]; 2],
            placement: None,
        });
        save(&mut document, &path).unwrap();
        let before = fs::read(&path).unwrap();
        let restored = open(&path, 16).unwrap();
        assert_eq!(restored.layers, document.layers);
        assert_eq!(restored.dpi, 144.0);
        assert!(!restored.dirty);
        assert_eq!(restored.history_counts(), (0, 0));
        let temporary = root.0.join("same.drawer.drawer-write.tmp");
        fs::write(&temporary, "other operation").unwrap();
        assert!(save(&mut document, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_to_string(&temporary).unwrap(), "other operation");
        fs::remove_file(temporary).unwrap();
        document.layers[0].pixels.clear();
        assert!(save(&mut document, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    #[test]
    fn rejects_truncation_bad_versions_and_oversized_documents() {
        let root = TempDir::new();
        let path = root.0.join("bad.drawer");
        save(&mut sample(), &path).unwrap();
        let original = fs::read(&path).unwrap();
        for bytes in [
            &original[..7],
            &original[..35],
            &original[..original.len() - 1],
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(open(&path, 16).is_err());
        }
        let mut bytes = original.clone();
        bytes[8..12].copy_from_slice(&(VERSION + 1).to_le_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(open(&path, 16).is_err());
        fs::write(&path, original).unwrap();
        assert!(open(&path, 7).is_err());
    }
    #[test]
    fn version_one_files_still_open_and_invalid_geometry_is_not_saved() {
        let root = TempDir::new();
        let path = root.0.join("legacy.drawer");
        let mut doc = sample();
        save(&mut doc, &path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let placement = 32 + 4 + doc.layers[0].name.len();
        bytes.drain(placement..placement + 8);
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes.truncate(32 + 4 + "Layer 1".len() + 8 + 8);
        fs::write(&path, &bytes).unwrap();
        let reopened = open(&path, 16).unwrap();
        assert_eq!(doc.snapshot(), reopened.snapshot());
        doc.scene.transform.0[0] = f64::NAN;
        assert!(save(&mut doc, &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn layer_locks_selection_translation_round_trip_and_version_six_is_unlocked() {
        let root = TempDir::new();
        let path = root.0.join("locked.drawer");
        let mut doc = sample();
        doc.layers[0].locked = true;
        doc.scene.selection_translation = [1.0, 10.0];
        save(&mut doc, &path).unwrap();
        assert!(open(&path, 16).unwrap().snapshot() == doc.snapshot());
        let mut bytes = fs::read(&path).unwrap();
        let tag = 32 + 4 + doc.layers[0].name.len() + 4;
        bytes.drain(tag..tag + 4);
        bytes[8..12].copy_from_slice(&6u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let legacy = open(&path, 16).unwrap();
        assert!(!legacy.layers[0].locked);
        assert_eq!(legacy.layers[0].pixels, doc.layers[0].pixels);
        assert_eq!(legacy.scene.selection_translation, [1.0, 10.0]);
    }
    #[test]
    fn version_seven_without_mouse_records_preserves_existing_lock_and_geometry() {
        let root = TempDir::new();
        let path = root.0.join("version7.drawer");
        let mut doc = sample();
        doc.layers[0].locked = true;
        save(&mut doc, &path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&7u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(open(&path, 16).unwrap().snapshot() == doc.snapshot());
    }
    #[test]
    fn version_two_geometry_without_grid_loads_with_default_grid() {
        let root = TempDir::new();
        let path = root.0.join("legacy2.drawer");
        let mut doc = sample();
        save(&mut doc, &path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let placement = 32 + 4 + doc.layers[0].name.len();
        bytes.drain(placement..placement + 8);
        let offset = 32 + 4 + doc.layers[0].name.len() + 8 + doc.layers[0].pixels.len() * 4;
        let mut scene = serde_json::to_value(&doc.scene).unwrap();
        scene.as_object_mut().unwrap().remove("grid");
        let json = serde_json::to_vec(&scene).unwrap();
        bytes.truncate(offset);
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&json);
        fs::write(&path, bytes).unwrap();
        assert_eq!(open(&path, 16).unwrap().snapshot(), doc.snapshot());
    }
    #[test]
    fn png_import_preserves_original_and_starts_as_an_unsaved_document() {
        let root = TempDir::new();
        let path = root.0.join("picture.png");
        let rgba = vec![1, 2, 3, 255, 0, 0, 0, 0];
        image::save_buffer_with_format(
            &path,
            &rgba,
            2,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let document = open(&path, 1024 * 1024).unwrap();
        assert_eq!(document.layers[0].pixels, [[1, 2, 3, 255], [0; 4]]);
        assert!(document.dirty && document.path.is_none());
        assert!(document.source_path.is_some());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(open(&path, 1).is_err());
    }
}
