use crate::{
    bucket::{self, Reference, Settings},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    fill_reference::Filter,
    layer_catalog as c,
    layer_properties::{self as p, Target},
    layer_roles::{self as core, Flags},
    layer_search::Key,
    render::Renderer,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn page() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn apply(d: &mut Document, key: Key, f: Flags, out: Option<bool>) -> Result<(), String> {
    let s = core::prepare(d, key, f, out)?;
    d.commit_state(s)?;
    Ok(())
}
pub fn wall() -> Result<Document, String> {
    let mut d = page()?;
    let s = crate::layers::prepare(
        &d,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "hidden wall".into(),
            parent: None,
        },
    )?
    .state;
    d.commit_state(s)?;
    d.layers[1].placement = None;
    d.layers[1].pixels = vec![[0; 4]; 64];
    for i in 1..7 {
        for (x, y) in [(i, 1), (i, 6), (1, i), (6, i)] {
            d.layers[1].pixels[y * 8 + x] = [0, 0, 0, 255];
        }
    }
    apply(
        &mut d,
        Key::Layer(Target::Raster(2)),
        Flags {
            reference: true,
            draft: false,
        },
        None,
    )?;
    let s = p::prepare_patch(
        &d,
        &[Target::Raster(2)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(s)?;
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = page()?;
    d.layers[0].pixels.fill([30, 80, 140, 128]);
    let key = Key::Layer(Target::Raster(1));
    let original = d.snapshot();
    let history = d.history_counts();
    let s = core::prepare(
        &d,
        key,
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    check(
        d.snapshot() == original && d.history_counts() == history,
        "Role draft changed source",
    )?;
    d.commit_state(s)?;
    check(
        Renderer::new(&d).pixel([3.5; 2]) == [30, 80, 140, 128]
            && d.layers[0].pixels == original.layers[0].pixels,
        "Role flags changed canvas/source",
    )?;
    check(
        Renderer::for_export(&d)?.pixel([3.5; 2]) == [0; 4],
        "Draft leaked into rendered output",
    )?;
    let saved = d.snapshot();
    let history = d.history_counts();
    let png = root.join("draft-excluded.png");
    crate::png_export::export(&d, &png, 1024 * 1024).map_err(|e| e.to_string())?;
    let image = image::ImageReader::open(&png)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .into_rgba8();
    check(
        image.pixels().all(|p| p.0 == [0; 4]),
        "PNG included draft pixels",
    )?;
    let psd = root.join("draft-excluded.psd");
    crate::psd::export(
        &d,
        &psd,
        crate::psd::Choice::RenderedImage,
        false,
        1024 * 1024,
    )
    .map_err(|e| e.to_string())?;
    let imported = crate::psd::open(&psd, 1024 * 1024).map_err(|e| e.to_string())?;
    check(
        imported.layers[0].pixels.iter().all(|p| *p == [0; 4])
            && d.snapshot() == saved
            && d.history_counts() == history,
        "PSD draft exclusion modified document",
    )?;
    check(
        crate::psd::export(
            &d,
            &root.join("draft-layer-rejected.psd"),
            crate::psd::Choice::RenderedLayers,
            false,
            1024 * 1024,
        )
        .is_err(),
        "Draft-excluding PSD layer export silently retained drafts",
    )?;
    let native = root.join("draft-preserved.drawer");
    crate::files::save(&mut d, &native).map_err(|e| e.to_string())?;
    let reopened =
        crate::files::open(&native, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot()
            && core::flags(&reopened, Target::Raster(1)).draft
            && Renderer::new(&reopened).pixel([3.5; 2]) == [30, 80, 140, 128],
        "Normal save discarded draft content/flags",
    )?;
    apply(
        &mut d,
        key,
        Flags {
            reference: true,
            draft: true,
        },
        Some(true),
    )?;
    check(
        Renderer::for_export(&d)?.pixel([3.5; 2]) == [30, 80, 140, 128],
        "Explicit draft output inclusion",
    )?;
    check(d.undo() && d.snapshot() == saved, "Output policy Undo")?;
    let d = wall()?;
    let before = d.snapshot();
    let history = d.history_counts();
    let s = Settings {
        reference: Reference::Marked,
        antialias: false,
        ..Default::default()
    };
    for (hidden, outside) in [(false, 255), (true, 0)] {
        let s = Settings {
            include_hidden: hidden,
            ..s
        };
        let region = bucket::Region::build(&d, 0, [3.5; 2], s)?;
        check(
            region.coverage(3, 3) == 255 && region.coverage(0, 0) == outside,
            "Hidden reference region policy",
        )?;
        let selected = crate::selection_edit::wand(
            &d,
            0,
            [3.5; 2],
            crate::selection_edit::Settings {
                region: s,
                ..Default::default()
            },
        )?;
        check(
            selected.selection_coverage([0.5; 2]) == outside
                && selected.selection_coverage([3.5; 2]) == 255,
            "Wand hidden-reference policy differs from fill",
        )?;
        let filled = bucket::prepare(&d, 0, Some([3.5; 2]), [200, 40, 20, 255], false, false, s)?;
        check(
            filled.layers[0].pixels[0][3] == outside
                && filled.layers[0].pixels[3 * 8 + 3] == [200, 40, 20, 255]
                && filled.layers[1].pixels == before.layers[1].pixels,
            "Hidden reference fill changed reference or escaped region",
        )?;
    }
    check(
        d.snapshot() == before
            && d.history_counts() == history
            && !p::properties(&d, Target::Raster(1)).hidden
            && p::properties(&d, Target::Raster(2)).hidden,
        "Reference sampling changed visibility/history",
    )?;
    let mut d = d;
    apply(
        &mut d,
        Key::Layer(Target::Raster(2)),
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    let region = bucket::Region::build(
        &d,
        0,
        [3.5; 2],
        Settings {
            include_hidden: true,
            exclude: Filter {
                draft: true,
                ..Default::default()
            },
            ..s
        },
    )?;
    check(
        region.coverage(0, 0) == 255,
        "Hidden reference inclusion bypassed independent draft exclusion",
    )?;
    folder_and_sources()?;
    Ok(())
}
fn folder_and_sources() -> Result<(), String> {
    use crate::animation::{Cell, Exposure, Timeline, Track};
    use std::sync::Arc;
    let mut animation = page()?;
    let mut state = animation.snapshot();
    state.animation = Some(Timeline {
        fps: Default::default(),
        frames: 2,
        audio: vec![],
        tracks: vec![Track {
            name: "reference animation".into(),
            cells: vec![Cell {
                name: "blue".into(),
                width: 1,
                height: 1,
                offset: [3; 2],
                pixels: Arc::new(vec![[0, 0, 255, 255]]),
            }],
            exposures: vec![Exposure { frame: 0, cell: 0 }],
            end: 2,
        }],
    });
    animation.commit_state(state)?;
    let key = Key::Track(animation.scene.catalog.tracks.id(0));
    apply(
        &mut animation,
        key,
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    let settings = Settings {
        reference: Reference::Marked,
        ..Default::default()
    };
    check(
        core::flags(&animation, Target::Cell { track: 0, cell: 0 }).draft
            && Renderer::new(&animation).pixel([3.5; 2]) == [0, 0, 255, 255]
            && Renderer::for_export(&animation)?.pixel([3.5; 2]) == [0; 4]
            && Renderer::fill(&animation, 0, settings)?.pixel([3.5; 2]) == [0, 0, 255, 255]
            && Renderer::fill(
                &animation,
                0,
                Settings {
                    exclude: Filter {
                        draft: true,
                        ..Default::default()
                    },
                    ..settings
                },
            )?
            .pixel([3.5; 2])
                == [0; 4],
        "Animation folder/cell role policy",
    )?;
    let mut d = page()?;
    d.layers[0].pixels.fill([50, 60, 70, 255]);
    let wrapped = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Wrap {
            name: "reference group".into(),
            targets: vec![Target::Raster(1)],
        },
    )?;
    d.commit_state(wrapped.state)?;
    let before = Renderer::new(&d).pixel([3.5; 2]);
    apply(
        &mut d,
        Key::Layer(Target::Folder(0)),
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    check(
        d.scene.paper.folders[0].through
            && core::flags(&d, Target::Raster(1)).reference
            && core::flags(&d, Target::Raster(1)).draft
            && Renderer::new(&d).pixel([3.5; 2]) == before
            && Renderer::for_export(&d)?.pixel([3.5; 2]) == [0; 4],
        "Through folder roles or inherited draft output",
    )?;
    check(
        c::rows(&d)?.iter().all(|r| r.states[5] && r.states[6]),
        "Search did not reflect inherited reference/draft flags",
    )?;
    let grouped = d.snapshot();
    let unwrapped =
        crate::layer_folders::prepare(&d, crate::layer_folders::Command::Unwrap(0))?.state;
    d.commit_state(unwrapped)?;
    check(
        core::own(&d, Target::Raster(1))
            == (Flags {
                reference: true,
                draft: true,
            })
            && Renderer::for_export(&d)?.pixel([3.5; 2]) == [0; 4],
        "Unwrap lost inherited reference/draft roles",
    )?;
    check(
        d.undo() && d.snapshot() == grouped,
        "Role-preserving unwrap Undo",
    )?;
    let normal = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Mode {
            folder: 0,
            through: false,
        },
    )?
    .state;
    d.commit_state(normal)?;
    crate::paper::bake(&mut d, crate::paper::Node::Folder(0))?;
    check(
        core::flags(&d, p::raster(&d, 0))
            == (Flags {
                reference: true,
                draft: true,
            })
            && Renderer::for_export(&d)?.pixel([3.5; 2]) == [0; 4],
        "Paper baking lost layer roles",
    )?;
    let mut mixed = page()?;
    mixed.layers[0].pixels.fill([1, 2, 3, 255]);
    let second = crate::layers::prepare(
        &mixed,
        crate::layers::Command::Create {
            kind: crate::layers::Kind::Raster,
            name: "different roles".into(),
            parent: None,
        },
    )?
    .state;
    mixed.commit_state(second)?;
    apply(
        &mut mixed,
        Key::Layer(Target::Raster(1)),
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    let before = mixed.snapshot();
    let op = || crate::layer_merge::Operation::Selected(vec![Target::Raster(1), Target::Raster(2)]);
    let result = crate::layer_merge::prepare(&mixed, op(), Default::default());
    check(
        result.err().is_some_and(|s| s.contains("roles")) && mixed.snapshot() == before,
        "Mixed layer roles were silently merged",
    )?;
    apply(
        &mut mixed,
        Key::Layer(Target::Raster(2)),
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    let merged = crate::layer_merge::prepare(&mixed, op(), Default::default())?;
    let target = merged.selected;
    mixed.commit_state(merged.state)?;
    check(
        core::own(&mixed, target)
            == (Flags {
                reference: true,
                draft: true,
            })
            && Renderer::for_export(&mixed)?.pixel([3.5; 2]) == [0; 4],
        "Merge lost shared layer roles",
    )?;
    let mut object = page()?;
    let mut scene = object.scene.clone();
    scene.objects.push(crate::scene::PageObject::Line {
        start: [1., 3.],
        end: [6., 3.],
        width_px: 2.,
        color: [200, 0, 0, 255],
    });
    object.edit_scene(scene)?;
    let key = Key::Object(object.scene.catalog.objects.id(0));
    apply(
        &mut object,
        key,
        Flags {
            reference: true,
            draft: true,
        },
        None,
    )?;
    check(
        Renderer::new(&object).pixel([3., 3.])[3] > 0
            && Renderer::for_export(&object)?.pixel([3., 3.]) == [0; 4],
        "Page-source draft output policy",
    )?;
    let s = Settings {
        reference: Reference::Marked,
        ..Default::default()
    };
    check(
        Renderer::fill(&object, 0, s)?.pixel([3., 3.])[3] > 0
            && Renderer::fill(
                &object,
                0,
                Settings {
                    exclude: Filter {
                        draft: true,
                        ..Default::default()
                    },
                    ..s
                },
            )?
            .pixel([3., 3.])
                == [0; 4],
        "Page-source marked/draft reference policy",
    )?;
    let mut vector = page()?;
    let mut scene = vector.scene.clone();
    let id = scene.lines.add_layer("Hidden reference vector")?;
    scene
        .lines
        .add_path(id, vec![[1., 3.], [6., 3.]], false, 2., [0, 0, 0, 255])?;
    vector.edit_scene(scene)?;
    apply(
        &mut vector,
        Key::Layer(Target::Vector(id)),
        Flags {
            reference: true,
            draft: false,
        },
        None,
    )?;
    let state = p::prepare_patch(
        &vector,
        &[Target::Vector(id)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?;
    vector.commit_state(state)?;
    check(
        Renderer::fill(&vector, 0, s)?.pixel([3., 3.]) == [0; 4]
            && Renderer::fill(
                &vector,
                0,
                Settings {
                    include_hidden: true,
                    ..s
                },
            )?
            .pixel([3., 3.])[3]
                > 0,
        "Hidden vector reference policy",
    )?;
    let mut settings = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    settings.input.fills[0].include_hidden = true;
    let preset = crate::brush_presets::Preset::capture(
        &settings,
        crate::assets::BrushTip::Pen,
        "Hidden references",
    )?;
    let preset = crate::brush_presets::Preset::decode(&preset.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    check(
        preset.fill.include_hidden,
        "Hidden reference tool preset lost setting",
    )?;
    let old = serde_json::to_value(Settings::default()).map_err(|e| e.to_string())?;
    let mut old = old.as_object().ok_or("Settings object")?.clone();
    old.remove("include_hidden");
    check(
        !serde_json::from_value::<Settings>(old.into())
            .map_err(|e| e.to_string())?
            .include_hidden,
        "Legacy reference default changed",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn flags_reference_wand_output_native_and_inherited_roles() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
