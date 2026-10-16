//! Generated geometry only; no third-party document or binary fixtures.
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::{self as tools, Target},
    scene::Affine,
};
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let l = d.scene.lines.add_layer("Boundary")?;
    d.scene.lines.add_path(
        l,
        vec![[8.0, 8.0], [48.0, 8.0], [48.0, 36.0], [8.0, 36.0]],
        true,
        1.0,
        [0, 0, 0, 255],
    )?;
    d.scene.lines.settings.references = vec![Target::Vector(l)];
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    boundary()?;
    let mut d = erase_fixture()?;
    let before = d.snapshot();
    let revision = d.revision;
    let reference = d.scene.lines.layers[1].clone();
    if !tools::erase(&mut d, [30.0, 24.0], 2.0)?
        || d.revision != revision + 1
        || d.history_counts() != (1, 0)
        || d.scene.lines.layers[1] != reference
    {
        return Err("Multi-layer erase did not make one atomic edit".into());
    }
    let paths = &d.scene.lines.layers[0].paths;
    if paths.len() != 2
        || paths[0].points.last() != Some(&[20.0, 24.0])
        || paths[1].points.first() != Some(&[40.0, 24.0])
        || d.layers[0].pixel(64, 48, 30, 24)[3] != 0
        || d.layers[1].pixel(64, 48, 30, 24)[3] != 0
    {
        return Err("Intersection interval/raster erase mismatch".into());
    }
    let after = d.snapshot();
    if !d.undo() || d.snapshot() != before || !d.redo() || d.snapshot() != after {
        return Err("Multi-layer undo/redo mismatch".into());
    }
    let path = root.join("line-references.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    if reopened.snapshot() != after {
        return Err("Vector/settings native round trip mismatch".into());
    }
    for bad in [
        Target::Raster(99),
        Target::Vector(999),
        Target::Track(0),
        Target::PageObjects,
    ] {
        let mut d = erase_fixture()?;
        d.scene.lines.settings.erase.push(bad);
        let before = d.snapshot();
        if tools::erase(&mut d, [30.0, 24.0], 2.0).is_ok()
            || d.snapshot() != before
            || d.history_counts() != (0, 0)
        {
            return Err("Invalid multi-target operation partially applied".into());
        }
    }
    Ok(())
}
pub fn erase_fixture() -> Result<Document, String> {
    let mut d = fixture()?;
    d.scene.lines = Default::default();
    d.layers[0].pixels.fill([100, 80, 60, 200]);
    let mut l = d.layers[0].clone();
    l.name = "Second raster".into();
    d.layers.push(l);
    let target = d.scene.lines.add_layer("Actual target")?;
    d.scene.lines.add_path(
        target,
        vec![[4.0, 24.0], [60.0, 24.0]],
        false,
        1.0,
        [0, 0, 0, 255],
    )?;
    let reference = d.scene.lines.add_layer("Read-only crossing references")?;
    for x in [20.0, 40.0] {
        d.scene.lines.add_path(
            reference,
            vec![[x, 4.0], [x, 44.0]],
            false,
            1.0,
            [0, 0, 0, 255],
        )?;
    }
    d.scene.lines.layers[1].locked = true;
    let hidden = d.scene.lines.add_layer("Hidden coincident vector")?;
    d.scene.lines.add_path(
        hidden,
        vec![[4.0, 24.0], [60.0, 24.0]],
        false,
        1.0,
        [0, 0, 0, 255],
    )?;
    d.scene.lines.layers[2].hidden = true;
    let locked = d.scene.lines.add_layer("Locked coincident vector")?;
    d.scene.lines.add_path(
        locked,
        vec![[4.0, 24.0], [60.0, 24.0]],
        false,
        1.0,
        [0, 0, 0, 255],
    )?;
    d.scene.lines.layers[3].locked = true;
    d.scene.lines.settings.intersections = vec![reference];
    d.scene.lines.settings.erase =
        vec![Target::Raster(0), Target::Raster(1), Target::Vector(target)];
    Ok(d)
}
pub fn boundary() -> Result<(), String> {
    use crate::painting::Stroke;
    let mut d = fixture()?;
    let base = d.snapshot();
    crate::line_regions::fill(&mut d, 0, [24.0, 24.0], [200, 40, 20, 255])?;
    for y in 0..48 {
        for x in 0..64 {
            let alpha = d.layers[0].pixel(64, 48, x, y)[3];
            if (x <= 8 || x >= 47 || y <= 8 || y >= 35) && alpha != 0 {
                return Err("Fill crossed the reference boundary".into());
            }
        }
    }
    if d.layers[0].pixel(64, 48, 24, 24)[3] != 255 || !d.undo() || d.snapshot() != base {
        return Err("Bounded fill/undo mismatch".into());
    }
    d.scene.lines.settings.clip_paint = true;
    let brush = crate::painting::Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        shape: crate::brush_cursor::Shape {
            size: 24.0,
            aspect: 1.0,
            angle: 0.0,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [200, 40, 20, 255],
        opacity: 1.0,
        hardness: 1.0,
        spacing: 0.15,
        smoothing: 0.0,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    };
    let mut stroke = Stroke::begin(&d, 0, brush)?;
    stroke.sample(&d, [24.0, 24.0])?;
    stroke.sample(&d, [60.0, 24.0])?;
    stroke.commit(&mut d)?;
    if d.layers[0].pixel(64, 48, 24, 24)[3] == 0
        || d.layers[0].pixel(64, 48, 48, 24)[3] != 0
        || d.layers[0].pixel(64, 48, 60, 24)[3] != 0
    {
        return Err("Brush escaped its frozen reference region".into());
    }
    let before = d.snapshot();
    d.scene.lines.layers[0].paths[0].closed = false;
    let open = d.snapshot();
    if crate::line_regions::fill(&mut d, 0, [24.0, 24.0], [0, 0, 0, 255]).is_ok()
        || d.snapshot() != open
    {
        return Err("Open reference boundary accepted".into());
    }
    before.restore(&mut d);
    d.scene.lines.settings.magnet = true;
    d.scene.lines.settings.distance = 3.0;
    d.scene.lines.layers[0].locked = true;
    if tools::magnet(&d, [10.0, 20.0])? != [8.0, 20.0]
        || tools::magnet(&d, [12.0, 20.0])? != [12.0, 20.0]
    {
        return Err("Magnet doc-distance mismatch".into());
    }
    d.scene.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    if tools::magnet(&d, [18.0, 40.0])? != [16.0, 40.0]
        || tools::magnet(&d, [20.0, 40.0])? != [20.0, 40.0]
    {
        return Err("Transformed magnet distance mismatch".into());
    }
    d.scene.lines.layers[0].hidden = true;
    if tools::magnet(&d, [18.0, 40.0])? != [18.0, 40.0] {
        return Err("Hidden path captured magnet".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_fill_magnet_atomic_erase_and_native() {
        let t = crate::test_support::TempDir::new();
        verify(&t.0).unwrap();
    }
    #[test]
    fn hidden_locked_and_unsupported_targets_abort_every_layer() {
        for vector in [false, true] {
            for hidden in [false, true] {
                let mut d = erase_fixture().unwrap();
                if vector {
                    if hidden {
                        d.scene.lines.layers[0].hidden = true
                    } else {
                        d.scene.lines.layers[0].locked = true
                    }
                } else if hidden {
                    d.scene.paper.layers.entry(1).or_default().hidden = true
                } else {
                    d.layers[1].locked = true
                }
                let before = d.snapshot();
                assert!(tools::preflight(&d).iter().any(|(_, e)| e.is_some()));
                assert!(tools::erase(&mut d, [30.0, 24.0], 2.0).is_err());
                assert!(d.snapshot() == before);
                assert_eq!(d.history_counts(), (0, 0));
            }
        }
    }
    #[test]
    fn raster_boundary_folder_visibility_and_selection() {
        let mut d = fixture().unwrap();
        d.scene.lines = Default::default();
        let mut boundary = d.layers[0].clone();
        boundary.locked = true;
        for y in 8..37 {
            for x in 8..49 {
                if y == 8 || y == 36 || x == 8 || x == 48 {
                    boundary.pixels[y * 64 + x] = [0, 0, 0, 255];
                }
            }
        }
        d.layers.push(boundary);
        d.scene.lines.settings.references = vec![Target::Raster(1)];
        d.scene.selection = Some(crate::scene::Rect {
            min: [20.0, 20.0],
            max: [30.0, 30.0],
        });
        let reference = d.layers[1].clone();
        crate::line_regions::fill(&mut d, 0, [24.0, 24.0], [20, 60, 100, 128]).unwrap();
        assert_eq!(d.layers[0].pixel(64, 48, 24, 24), [20, 60, 100, 128]);
        assert_eq!(d.layers[0].pixel(64, 48, 15, 24), [0; 4]);
        assert!(d.layers[1] == reference);
        let folder = d.scene.paper.group_all(2, "Hidden parent").unwrap();
        d.scene.paper.folders[folder].paper.hidden = true;
        assert!(!d.scene.paper.raster_visible(1, 2));
        let before = d.snapshot();
        assert!(crate::line_regions::fill(&mut d, 0, [24.0, 24.0], [1; 4]).is_err());
        assert!(d.snapshot() == before);
    }
    #[test]
    fn closed_vector_wrap_and_multiple_vector_targets_restore_together() {
        let mut d = fixture().unwrap();
        let target = d.scene.lines.layers[0].id;
        let other = d.scene.lines.add_layer("Second target").unwrap();
        d.scene
            .lines
            .add_path(
                other,
                vec![[8.0, 8.0], [48.0, 8.0], [48.0, 36.0], [8.0, 36.0]],
                true,
                1.0,
                [0, 0, 0, 255],
            )
            .unwrap();
        let refs = d.scene.lines.add_layer("Crossings").unwrap();
        for x in [20.0, 40.0] {
            d.scene
                .lines
                .add_path(refs, vec![[x, 0.0], [x, 46.0]], false, 1.0, [0, 0, 0, 255])
                .unwrap();
        }
        d.scene.lines.settings.intersections = vec![refs];
        d.scene.lines.settings.erase = vec![Target::Vector(target), Target::Vector(other)];
        let before = d.snapshot();
        tools::erase(&mut d, [10.0, 8.0], 1.0).unwrap();
        assert_eq!(d.scene.lines.layers[0].paths.len(), 1);
        assert!(!d.scene.lines.layers[0].paths[0].hit([10.0, 8.0]));
        assert!(d.scene.lines.layers[0].paths[0].hit([30.0, 8.0]));
        assert!(d.undo());
        assert!(d.snapshot() == before);
    }
    #[test]
    fn invalid_data_and_legacy_defaults() {
        let mut data = tools::Data::default();
        let id = data.add_layer("Local").unwrap();
        let before = data.clone();
        assert!(
            data.add_path(id, vec![[f64::NAN, 0.0], [1.0, 1.0]], false, 1.0, [0; 4])
                .is_err()
        );
        assert_eq!(data, before);
        let scene = crate::scene::Scene::default();
        let mut value = serde_json::to_value(&scene).unwrap();
        value.as_object_mut().unwrap().remove("lines");
        assert_eq!(
            serde_json::from_value::<crate::scene::Scene>(value)
                .unwrap()
                .lines,
            tools::Data::default()
        );
    }
    #[test]
    fn late_vector_capacity_error_rolls_back_staged_raster_erase() {
        let mut d = erase_fixture().unwrap();
        let target = d.scene.lines.layers[0].id;
        while d
            .scene
            .lines
            .layers
            .iter()
            .map(|l| l.paths.len())
            .sum::<usize>()
            < 128
        {
            d.scene
                .lines
                .add_path(
                    target,
                    vec![[4.0, 24.0], [60.0, 24.0]],
                    false,
                    1.0,
                    [0, 0, 0, 255],
                )
                .unwrap();
        }
        let before = d.snapshot();
        assert!(tools::erase(&mut d, [30.0, 24.0], 2.0).is_err());
        assert!(d.snapshot() == before);
        assert_eq!(d.history_counts(), (0, 0));
    }
    #[test]
    fn renderer_visibility_page_transform_and_region_budget() {
        let mut d = fixture().unwrap();
        assert_eq!(
            crate::render::Renderer::new(&d).pixel([8.0, 20.0]),
            [0, 0, 0, 255]
        );
        d.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 80.0, 0.0]);
        assert_eq!(tools::magnet(&d, [39.0, 17.0]).unwrap(), [39.0, 17.0]);
        assert_eq!(
            crate::render::Renderer::new(&d).pixel([40.0, 16.0]),
            [0, 0, 0, 255]
        );
        d.scene.lines.layers[0].hidden = true;
        assert_eq!(crate::render::Renderer::new(&d).pixel([40.0, 16.0]), [0; 4]);
        let mut d = Document::create(
            CanvasSpec {
                width: 2049,
                height: 2048,
                dpi: 300.0,
                background: Background::Transparent,
            },
            DEFAULT_DOCUMENT_BUDGET,
        )
        .unwrap();
        let revision = d.revision;
        assert!(crate::line_regions::fill(&mut d, 0, [20.0, 20.0], [1; 4]).is_err());
        assert_eq!(d.revision, revision);
        assert_eq!(d.history_counts(), (0, 0));
        assert!(d.layers[0].pixels.iter().all(|p| *p == [0; 4]));
    }
    #[test]
    fn vector_preview_includes_the_exact_outer_edge() {
        let d = fixture().unwrap();
        let r = crate::render::Renderer::new(&d);
        assert_eq!(r.pixel([7.5, 20.5]), [0, 0, 0, 255]);
        assert_eq!(r.pixel([8.5, 20.5]), [0, 0, 0, 255]);
        assert_eq!(r.pixel([8.5001, 20.5]), [0; 4]);
    }
}
