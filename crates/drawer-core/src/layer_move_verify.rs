use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as c,
    layer_move::{self as m, Settings, Source},
    layer_properties::Target,
    layer_search::{Key, Kind},
    material_layer::{Layer, Source as Material},
    scene::{Affine, PageObject, Rect},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([50, 100, 150, 128]);
    d.layers[0].name = "Lower pick".into();
    let mut locked = d.layers[0].clone();
    locked.name = "Locked upper".into();
    locked.pixels.fill([0; 4]);
    locked.pixels[2 * 8 + 2] = [220, 40, 50, 255];
    locked.locked = true;
    d.layers.push(locked);
    let mut partial = d.layers[0].clone();
    partial.name = "Partial area".into();
    partial.pixels.fill([0; 4]);
    partial.pixels[4 * 8 + 5] = [80, 120, 200, 128];
    d.layers.push(partial);
    for source in [
        Material::Image(crate::material_layer::Image {
            width: 1,
            height: 1,
            pixels: std::sync::Arc::new(vec![[70, 180, 200, 255]]),
        }),
        Material::Solid([200, 100, 50, 255]),
    ] {
        d.scene.objects.push(PageObject::Material(Box::new(Layer {
            source,
            area: Rect {
                min: [0.; 2],
                max: [3.; 2],
            },
            transform: Affine([1., 0., 0., 1., 1., 1.]),
            repeat: [true; 2],
        })));
    }
    d.scene.objects.push(PageObject::Line {
        start: [1., 2.5],
        end: [4., 2.5],
        width_px: 1.,
        color: [255, 30, 50, 255],
    });
    d.scene.objects.push(PageObject::Text {
        baseline: [1., 6.],
        text: "Hi".into(),
        size_px: 5.,
        color: [255, 255, 255, 255],
    });
    let mut state = d.snapshot();
    state.scene.layer_ids.initialize(d.layers.len())?;
    c::reconcile(&d, &mut state)?;
    let mut info = state
        .scene
        .catalog
        .info(Key::Object(3))
        .cloned()
        .unwrap_or_default();
    info.kind = Some(Kind::Balloon);
    state.scene.catalog.set(Key::Object(3), info);
    d.commit_state(state)?;
    Ok(d)
}
pub fn click_settings() -> Settings {
    Settings {
        source: Source::Click,
        exclude_text: true,
        exclude_balloon: true,
        exclude_image: true,
        exclude_fill: true,
        ..Default::default()
    }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let r = Key::Layer(Target::Raster(1));
    let original = d.snapshot();
    let chosen = m::resolve(&d, r, [2.5, 2.5], click_settings())?;
    check(
        chosen == vec![r] && d.snapshot() == original,
        "Click exclusions/locked upper did not find lower raster",
    )?;
    let settings = Settings {
        exclude_image: false,
        ..click_settings()
    };
    check(
        m::resolve(&d, r, [2.5, 2.5], settings)? == vec![Key::Object(1)],
        "Image material exclusion toggle",
    )?;
    let settings = Settings {
        exclude_fill: false,
        ..click_settings()
    };
    check(
        m::resolve(&d, r, [2.5, 2.5], settings)? == vec![Key::Object(2)],
        "Fill layer exclusion toggle",
    )?;
    check(
        !m::eligible(&d, Key::Object(3), click_settings())
            && !m::eligible(&d, Key::Object(4), click_settings()),
        "Balloon/text typed exclusions",
    )?;
    let mut paper = crate::paper::Paper::default();
    paper.pattern.source = 1;
    d.scene.paper.layers.insert(0, paper);
    check(
        m::resolve(
            &d,
            r,
            [2.5, 2.5],
            Settings {
                exclude_texture: true,
                ..click_settings()
            },
        )
        .is_err(),
        "Texture exclusion ignored",
    )?;
    d.scene.paper.layers.remove(&0);
    d.scene.selection = Some(Rect {
        min: [4., 3.],
        max: [6., 5.],
    });
    let area = Settings {
        source: Source::Area,
        ..click_settings()
    };
    let chosen = m::resolve(&d, r, [0.; 2], area)?;
    check(
        chosen == vec![Key::Layer(Target::Raster(3)), r],
        "Area mode missed partial intersection or included locked/material/fill",
    )?;
    let source = d.snapshot();
    let preview = m::prepare(&d, &chosen, [2, 0], false)?;
    check(
        d.snapshot() == source && preview.selected == chosen[0],
        "Area movement preview changed original/primary",
    )?;
    d.commit_state(preview.state)?;
    check(
        d.layers[0].extent(8, 8).offset == [2, 0]
            && d.layers[2].extent(8, 8).offset == [2, 0]
            && d.layers[1] == source.layers[1]
            && d.scene.objects == source.scene.objects,
        "Area mode did not move whole layers or changed excluded content",
    )?;
    check(d.undo() && d.snapshot() == source, "Area movement Undo")?;
    let mut info = d.scene.catalog.rasters.get(&3).cloned().unwrap_or_default();
    info.draft = true;
    d.scene.catalog.rasters.insert(3, info);
    check(
        m::resolve(
            &d,
            r,
            [0.; 2],
            Settings {
                exclude_draft: true,
                ..area
            },
        )? == vec![r],
        "Draft exclusion ignored",
    )?;
    let old = d.snapshot();
    let counts = d.history_counts();
    let copied = m::prepare(&d, &chosen, [2, 1], true)?;
    let copy_key = copied.selected;
    check(
        d.snapshot() == old && copied.state.layers.len() == 5,
        "Copy movement preview changed original or did not copy both layers",
    )?;
    d.commit_state(copied.state)?;
    check(
        d.layers[..3] == old.layers[..]
            && d.history_counts().0 == counts.0 + 1
            && copy_key != chosen[0],
        "Copy changed originals or split Undo",
    )?;
    check(c::target(&d,copy_key).is_some_and(|t|matches!(t,Target::Raster(id)if crate::layers::index(&d,id).is_some_and(|n|d.layers[n].extent(8,8).offset==[2,1]))),"Copy result selection/position")?;
    let path = root.join("move-copies.drawer");
    let after = d.snapshot();
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == after,
        "Copy movement native roundtrip",
    )?;
    check(d.undo() && d.snapshot() == old, "Copy movement Undo")?;
    let noop = m::prepare(&d, &chosen, [0; 2], true)?;
    check(
        noop.state == d.snapshot(),
        "Zero-distance copy created layers",
    )?;
    // A locked or excluded linked member prevents every member from starting.
    crate::layer_groups_verify::save(&mut d, vec![r, Key::Layer(Target::Raster(2))])?;
    let original = d.snapshot();
    check(
        m::resolve(&d, r, [2.5, 2.5], click_settings()).is_err()
            && m::prepare(&d, &[r], [1, 0], true).is_err()
            && d.snapshot() == original,
        "Locked linked copy partially changed page",
    )?;
    let mut d = crate::layer_groups_verify::fixture()?;
    let keys = crate::layer_groups_verify::keys(&d);
    crate::layer_groups_verify::save(&mut d, keys.clone())?;
    let original = d.snapshot();
    let copied = m::prepare(&d, &[keys[0]], [1, 2], true)?;
    d.commit_state(copied.state)?;
    check(
        d.scene.layer_groups.groups.len() == 2
            && d.scene.rulers.guides.len() == 2
            && d.layers[..original.layers.len()] == original.layers[..]
            && d.scene.lines.layers[0] == original.scene.lines.layers[0],
        "Linked copy lost group/ruler or changed original ink",
    )?;
    check(
        d.undo() && d.snapshot() == original,
        "Linked group copy Undo",
    )?;
    // A layer-scoped guide without a movement link keeps its fixed geometry.
    let mut scoped = crate::layer_groups_verify::fixture()?;
    let vector = scoped.scene.lines.layers[0].id;
    let target = crate::line_tools::Target::Vector(vector);
    scoped.scene.rulers.guides[0].link = None;
    scoped.scene.rulers.guides[0].scope = crate::ruler_scope::Scope::Layer(target);
    let guide = scoped.scene.rulers.guides[0].clone();
    let copy = m::prepare(&scoped, &[Key::Layer(Target::Vector(vector))], [2, 0], true)?;
    scoped.commit_state(copy.state)?;
    let new_vector = scoped.scene.lines.layers[1].id;
    check(
        scoped.scene.rulers.guides.len() == 2
            && scoped.scene.rulers.guides[0] == guide
            && scoped.scene.rulers.guides[1].link.is_none()
            && scoped.scene.rulers.guides[1].scope
                == crate::ruler_scope::Scope::Layer(crate::line_tools::Target::Vector(new_vector))
            && scoped.scene.rulers.guides[1].kind == guide.kind
            && scoped.scene.rulers.guides[1].space == guide.space,
        "Unlinked layer-scoped guide lost copied scope or moved its fixed geometry",
    )?;
    // Folder copies retain all child data and move the new subtree only.
    let folder = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Wrap {
            name: "Copy subtree".into(),
            targets: vec![Target::Raster(2)],
        },
    )?;
    let f = folder.selected;
    d.commit_state(folder.state)?;
    let original = d.snapshot();
    let copy = m::prepare(&d, &[Key::Layer(f)], [1, 0], true)?;
    let selected = copy.selected;
    d.commit_state(copy.state)?;
    check(
        matches!(selected,Key::Layer(Target::Folder(n))if n!=0)
            && d.layers[..original.layers.len()] == original.layers[..]
            && d.scene.paper.folders[0] == original.scene.paper.folders[0],
        "Folder copy changed source subtree",
    )?;
    // Selected-pixel copy keeps the source and copies selection strength once.
    let mut d = crate::layer_masks_verify::fixture()?;
    let original = d.snapshot();
    let copy = m::copy_selected_pixels(&d, 0, [2, 1])?;
    d.commit_state(copy.state)?;
    check(
        d.layers.len() == 2
            && d.layers[0] == original.layers[0]
            && d.layers[1].pixel(3, 1, 3, 1) == [100, 150, 200, 64]
            && d.layers[1].pixel(3, 1, 4, 1) == [100, 150, 200, 128]
            && d.scene.selection_translation == [2., 1.],
        "Selected copy changed source or applied selection twice",
    )?;
    check(d.undo() && d.snapshot() == original, "Selected copy Undo")?;
    let settings = crate::workspaces::Snapshot {
        layer_move: Settings {
            source: Source::Area,
            auto_select: false,
            keep_original: true,
            exclude_draft: true,
            ..click_settings()
        },
        ..Default::default()
    };
    let path = root.join("layer-move-workspace.json");
    settings.export(&path).map_err(|e| e.to_string())?;
    let loaded = crate::workspaces::Snapshot::read(&path).map_err(|e| e.to_string())?;
    check(
        loaded.layer_move == settings.layer_move,
        "Movement local settings restart",
    )?;
    let (base, _) = crate::workspaces::Snapshot::default()
        .merge(
            &loaded,
            crate::workspaces::Scope {
                layout: false,
                bars: false,
                keys: false,
                other: true,
            },
            &[],
        )
        .map_err(|e| e.to_string())?;
    check(
        base.layer_move == loaded.layer_move
            && crate::workspaces::Snapshot::default().layer_move == Settings::default(),
        "Movement settings import/reset",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn layer_targeting_exclusions_area_partial_copy_groups_native_preferences() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
