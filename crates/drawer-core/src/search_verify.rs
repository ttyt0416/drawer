//! Shared installed-build regression for stable search edits and canvas selection.
use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_catalog as c,
    layer_properties::{self as p, Target},
    layer_search::{self as q, Key, Selection, Settings},
    layers::{self, Command, Kind},
    search_edit::{self, Edit},
};
pub fn page() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn apply(d: &mut Document, s: &Settings, selection: &Selection, e: Edit) -> Result<(), String> {
    d.commit_state(search_edit::prepare(d, s, selection, e)?.state)?;
    Ok(())
}
fn chosen(keys: &[Key]) -> Selection {
    Selection {
        keys: keys.iter().copied().collect(),
        primary: keys.first().copied(),
        anchor: keys.first().copied(),
    }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    timeline()?;
    let mut d = page()?;
    for name in ["keep", "wanted", "excluded"] {
        let s = layers::prepare(
            &d,
            Command::Create {
                kind: Kind::Raster,
                name: name.into(),
                parent: None,
            },
        )?
        .state;
        d.commit_state(s)?;
    }
    let keys = [Key::Layer(Target::Raster(3)), Key::Layer(Target::Raster(4))];
    let selection = chosen(&keys);
    let settings = Settings {
        name: "wanted".into(),
        ..Default::default()
    };
    let before = d.snapshot();
    let revision = d.revision;
    let result = q::query(
        &c::rows(&d)?,
        &settings,
        selection.primary,
        d.scene.catalog.expression,
    )?;
    check(
        result == vec![keys[0]] && d.snapshot() == before && d.revision == revision,
        "Search changed document or literal results",
    )?;
    apply(
        &mut d,
        &settings,
        &selection,
        Edit::Properties(p::Patch {
            hidden: Some(true),
            ..Default::default()
        }),
    )?;
    check(
        p::properties(&d, Target::Raster(3)).hidden && !p::properties(&d, Target::Raster(4)).hidden,
        "Filtered-out selection was hidden",
    )?;
    check(
        d.undo() && d.snapshot() == before,
        "Search visibility is not one Undo",
    )?;
    apply(&mut d, &settings, &selection, Edit::Delete)?;
    check(
        layers::index(&d, 3).is_none() && layers::index(&d, 4).is_some() && d.layers.len() == 3,
        "Filtered-out selected layer was deleted",
    )?;
    check(d.undo() && d.snapshot() == before, "Search deletion Undo")?;
    apply(
        &mut d,
        &Settings::default(),
        &chosen(&[keys[0]]),
        Edit::Tag {
            value: "Straße".into(),
            remove: false,
        },
    )?;
    check(
        q::query(
            &c::rows(&d)?,
            &Settings {
                tag: "STRASSE".into(),
                ..Default::default()
            },
            None,
            c::Data::default().expression,
        )? == vec![keys[0]],
        "Unicode tags/full case fold",
    )?;
    let mut scene = d.scene.clone();
    for y in [3., 6.] {
        scene.objects.push(crate::scene::PageObject::Line {
            start: [1., y],
            end: [12., y],
            width_px: 2.,
            color: [255, 0, 0, 255],
        });
    }
    d.edit_scene(scene)?;
    let id = d.scene.catalog.objects.id(1);
    let object = Key::Object(id);
    apply(
        &mut d,
        &Settings::default(),
        &chosen(&[object]),
        Edit::Rename("survivor".into()),
    )?;
    let removed = Key::Object(d.scene.catalog.objects.id(0));
    apply(
        &mut d,
        &Settings::default(),
        &chosen(&[removed]),
        Edit::Delete,
    )?;
    check(
        c::source_index(&d, object) == Some(0)
            && c::rows(&d)?
                .iter()
                .any(|r| r.key == object && r.name == "survivor"),
        "Page object identity changed after deletion",
    )?;
    let before = d.snapshot();
    let moved = crate::search_move::prepare(&d, object, [2, 1])?;
    d.commit_state(moved)?;
    check(
        crate::layer_pick::candidates(&d, [6., 7.], Default::default()).first()
            == Some(&crate::thumbnail::Target::Source(object)),
        "Moved object not directly selectable",
    )?;
    check(
        d.undo() && d.snapshot() == before,
        "Picked source movement Undo",
    )?;
    apply(
        &mut d,
        &Settings::default(),
        &chosen(&[object]),
        Edit::Properties(p::Patch {
            locked: Some(true),
            ..Default::default()
        }),
    )?;
    check(
        crate::layer_pick::candidates(&d, [6., 6.], Default::default())
            .iter()
            .all(|t| *t != crate::thumbnail::Target::Source(object))
            && crate::layer_pick::candidates(
                &d,
                [6., 6.],
                crate::layer_pick::Settings {
                    include_locked: true,
                    ..Default::default()
                },
            )
            .first()
                == Some(&crate::thumbnail::Target::Source(object)),
        "Locked canvas picking policy",
    )?;
    check(
        crate::search_move::prepare(&d, object, [1, 0]).is_err(),
        "Picked locked object moved",
    )?;
    let path = root.join("layer-search.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let reopened = crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    check(
        reopened.snapshot() == d.snapshot() && c::source_index(&reopened, object) == Some(0),
        "Search identities/properties native reopen",
    )?;
    Ok(())
}
fn timeline() -> Result<(), String> {
    use crate::animation::{Cell, Exposure, Timeline, Track};
    use std::sync::Arc;
    let mut d = page()?;
    let cell = |name: &str| Cell {
        name: name.into(),
        width: 1,
        height: 1,
        offset: [0; 2],
        pixels: Arc::new(vec![[1, 2, 3, 255]]),
    };
    let track = |name: &str| Track {
        name: name.into(),
        cells: vec![cell("unused"), cell("exposed")],
        exposures: vec![Exposure { frame: 0, cell: 1 }],
        end: 3,
    };
    let mut state = d.snapshot();
    state.animation = Some(Timeline {
        fps: Default::default(),
        frames: 3,
        tracks: vec![track("remove"), track("keep")],
        audio: vec![],
    });
    d.commit_state(state)?;
    let t1 = d.scene.catalog.tracks.id(0);
    let t2 = d.scene.catalog.tracks.id(1);
    let c1 = d.scene.catalog.cell_id(t1, 0);
    let c2 = d.scene.catalog.cell_id(t1, 1);
    let keep = d.scene.catalog.cell_id(t2, 1);
    let state = p::prepare_patch(
        &d,
        &[Target::Track(1), Target::Cell { track: 1, cell: 1 }],
        p::Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?;
    d.commit_state(state)?;
    let saved = d.snapshot();
    let s = chosen(&[
        Key::Track(t1),
        Key::Cell {
            track: t1,
            cell: c1,
        },
        Key::Cell {
            track: t1,
            cell: c2,
        },
    ]);
    let only_folder = Settings {
        kinds: q::Kinds::Some([q::Kind::AnimationFolder].into_iter().collect()),
        ..Default::default()
    };
    check(
        search_edit::prepare(&d, &only_folder, &s, Edit::Delete).is_err() && d.snapshot() == saved,
        "Filtered-out animation cells deleted with folder",
    )?;
    apply(&mut d, &Settings::default(), &s, Edit::Delete)?;
    check(
        c::target(&d, Key::Track(t2)) == Some(Target::Track(0))
            && c::target(
                &d,
                Key::Cell {
                    track: t2,
                    cell: keep,
                },
            ) == Some(Target::Cell { track: 0, cell: 1 })
            && p::properties(&d, Target::Track(0)).locked
            && p::properties(&d, Target::Cell { track: 0, cell: 1 }).locked,
        "Unselected locked track/cell identity or attributes changed after deletion",
    )?;
    check(
        d.undo() && d.snapshot() == saved,
        "Animation search batch Undo",
    )?;
    let s = chosen(&[Key::Cell {
        track: t1,
        cell: c1,
    }]);
    apply(&mut d, &Settings::default(), &s, Edit::Delete)?;
    check(
        c::target(
            &d,
            Key::Cell {
                track: t1,
                cell: c2,
            },
        ) == Some(Target::Cell { track: 0, cell: 0 })
            && d.animation
                .as_ref()
                .is_some_and(|a| a.tracks[0].exposures[0].cell == 0),
        "Deleting unused cell did not preserve exposed cell identity",
    )?;
    check(
        search_edit::prepare(
            &d,
            &Settings::default(),
            &chosen(&[Key::Cell {
                track: t1,
                cell: c2,
            }]),
            Edit::Delete,
        )
        .is_err(),
        "Exposed/last cell deletion accepted",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use q::{Condition as C, Context as X, Expression as E, Kind as K, Kinds, Row, Scope};
    fn row(n: u64, k: K, parent: Option<Key>) -> Row {
        Row {
            key: Key::Object(n),
            name: format!("layer {n}"),
            kind: k,
            expression: E::Color,
            extras: Default::default(),
            tags: vec![],
            states: [false; 7],
            parent,
        }
    }
    fn query(rows: &[Row], s: &Settings, k: Option<Key>) -> Vec<Key> {
        q::query(rows, s, k, E::Gray).unwrap()
    }
    #[test]
    fn every_kind_and_attachment_union_name_state_intersection() {
        let mut rows = K::ALL
            .iter()
            .enumerate()
            .map(|(i, k)| row(i as u64 + 1, *k, None))
            .collect::<Vec<_>>();
        for r in &mut rows {
            r.expression = E::Mono;
        }
        let mut s = Settings::default();
        for k in K::ALL {
            s.kinds = Kinds::Some([*k].into_iter().collect());
            assert_eq!(query(&rows, &s, None).len(), 1, "{k:?}");
        }
        rows[0].extras.insert(K::Mask);
        rows[3].extras.insert(K::Mask);
        rows[3].states[0] = true;
        s.kinds = Kinds::Some([K::RasterColor, K::Mask].into_iter().collect());
        assert_eq!(
            query(&rows, &s, None),
            [rows[0].key, rows[3].key, rows[21].key]
        );
        s.states[0] = C::Include;
        assert_eq!(query(&rows, &s, None), [rows[3].key]);
        s.name = "no such layer".into();
        assert!(query(&rows, &s, None).is_empty());
        s.name.clear();
        s.kinds = Kinds::All;
        assert_eq!(query(&rows, &s, None), [rows[3].key]);
        s.kinds = Kinds::Some(Default::default());
        assert!(query(&rows, &s, None).is_empty());
        let mut r = row(100, K::RasterColor, None);
        r.expression = E::Color;
        s = Settings {
            kinds: Kinds::Some([K::Broader].into_iter().collect()),
            ..Default::default()
        };
        assert_eq!(query(&[r], &s, None), [Key::Object(100)]);
    }
    #[test]
    fn seven_tristates_are_independent_and_conjunctive() {
        let mut a = row(1, K::RasterColor, None);
        a.states = [true; 7];
        let b = row(2, K::RasterColor, None);
        for n in 0..7 {
            let mut s = Settings::default();
            s.states[n] = C::Include;
            assert_eq!(query(&[a.clone(), b.clone()], &s, None), [a.key]);
            s.states[n] = C::Exclude;
            assert_eq!(query(&[a.clone(), b.clone()], &s, None), [b.key]);
            s.states[(n + 1) % 7] = C::Include;
            assert!(query(&[a.clone(), b.clone()], &s, None).is_empty());
        }
    }
    #[test]
    fn nested_folder_panel_animation_and_absent_outside_context() {
        let a = row(1, K::Folder, None);
        let b = row(2, K::PanelFolder, Some(a.key));
        let c = row(3, K::RasterGray, Some(b.key));
        let t = row(4, K::AnimationFolder, None);
        let cell = row(5, K::Cell, Some(t.key));
        let rows = [a, b, c, t, cell];
        for (context, key, inside) in [
            (X::Folder, rows[2].key, vec![rows[1].key, rows[2].key]),
            (X::Panel, rows[2].key, vec![rows[1].key, rows[2].key]),
            (X::Animation, rows[4].key, vec![rows[3].key, rows[4].key]),
        ] {
            let mut s = Settings {
                scope: Scope::Inside(context),
                ..Default::default()
            };
            assert_eq!(query(&rows, &s, Some(key)), inside);
            s.scope = Scope::Outside(context);
            let outside = query(&rows, &s, Some(key));
            assert_eq!(outside.len() + inside.len(), rows.len());
            assert!(outside.iter().all(|k| !inside.contains(k)));
            assert!(query(&rows, &s, Some(Key::Object(999))).is_empty());
            assert!(query(&rows, &s, None).is_empty());
        }
    }
    #[test]
    fn unicode_normalization_full_folding_and_literal_whitespace_wildcards() {
        let mut r = row(1, K::Text, None);
        r.name = "한글 Straße Σςσ Café *? two  spaces".into();
        for name in [
            "한글",
            "STRASSE",
            "σσσ",
            "CAFE\u{301}",
            "*?",
            "two  spaces",
            "",
        ] {
            assert_eq!(
                query(
                    &[r.clone()],
                    &Settings {
                        name: name.into(),
                        ..Default::default()
                    },
                    None
                ),
                [r.key],
                "{name}"
            );
        }
        for name in ["two spaces", "[a-z]", "^", "*?*"] {
            assert!(
                query(
                    &[r.clone()],
                    &Settings {
                        name: name.into(),
                        ..Default::default()
                    },
                    None
                )
                .is_empty()
            );
        }
    }
    #[test]
    fn range_selection_retains_hidden_keys_deletion_and_settings_roundtrip() {
        let rows = (1..=5).map(|n| row(n, K::Text, None)).collect::<Vec<_>>();
        let all = rows.iter().map(|r| r.key).collect::<Vec<_>>();
        let mut s = Selection::default();
        s.click(&all, all[1], false, false);
        s.click(&all, all[4], false, true);
        assert_eq!(s.keys.len(), 4);
        assert_eq!(s.visible(&all[3..]), all[3..]);
        s.click(&all, all[4], true, false);
        assert_eq!(s.keys.len(), 3);
        s.retain(&rows[..3]);
        assert_eq!(s.keys.len(), 2);
        let settings = Settings {
            name: "* 한글".into(),
            kinds: Kinds::Some([K::Mask, K::RasterGray].into_iter().collect()),
            scope: Scope::Outside(X::Folder),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_slice::<Settings>(&serde_json::to_vec(&settings).unwrap()).unwrap(),
            settings
        );
    }
    #[test]
    fn mixed_edits_stable_ids_native_roundtrip() {
        let root = crate::test_support::TempDir::new();
        verify(&root.0).unwrap();
    }
    #[test]
    fn real_catalog_masks_rulers_nested_scopes_refresh_and_folder_movement() {
        use crate::{layer_folders as f, scene::Rect};
        let mut d = page().unwrap();
        for name in ["inside", "nested"] {
            let s = layers::prepare(
                &d,
                Command::Create {
                    kind: Kind::Raster,
                    name: name.into(),
                    parent: None,
                },
            )
            .unwrap()
            .state;
            d.commit_state(s).unwrap();
        }
        for targets in [
            vec![Target::Raster(3)],
            vec![Target::Raster(2), Target::Folder(0)],
        ] {
            let s = f::prepare(
                &d,
                f::Command::Wrap {
                    name: "folder".into(),
                    targets,
                },
            )
            .unwrap()
            .state;
            d.commit_state(s).unwrap();
        }
        let state = layers::prepare(
            &d,
            Command::Create {
                kind: Kind::Vector,
                name: "masked vector".into(),
                parent: None,
            },
        )
        .unwrap()
        .state;
        d.commit_state(state).unwrap();
        let vector = d.scene.lines.layers[0].id;
        let mut scene = d.scene.clone();
        scene.catalog.set(
            Key::Layer(Target::Folder(0)),
            c::Info {
                kind: Some(K::PanelFolder),
                ..Default::default()
            },
        );
        scene.catalog.set(
            Key::Layer(Target::Raster(2)),
            c::Info {
                expression: E::Gray,
                tags: vec!["art".into()],
                ..Default::default()
            },
        );
        let mask = crate::layer_folders::rectangle_mask(
            &d,
            Rect {
                min: [0.; 2],
                max: [10.; 2],
            },
            128,
        )
        .unwrap();
        scene.layer_effects.vectors.insert(
            vector,
            crate::layer_effects::Effect {
                mask: Some(mask),
                ..Default::default()
            },
        );
        scene.rulers.next_id = 2;
        scene.rulers.guides.push(crate::rulers::Guide {
            space: Default::default(),
            enabled: true,
            display: Default::default(),
            scope: crate::ruler_scope::Scope::Folder(1),
            frames: None,
            link: None,
            offset: [0.; 2],
            id: 1,
            name: "folder ruler".into(),
            kind: crate::rulers::Kind::Straight {
                a: [0.; 2],
                b: [4., 0.],
            },
        });
        d.edit_scene(scene).unwrap();
        let rows = c::rows(&d).unwrap();
        let s = Settings {
            kinds: Kinds::Some([K::RasterColor, K::Mask].into_iter().collect()),
            ..Default::default()
        };
        assert_eq!(
            query(&rows, &s, None)
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            [
                Key::Layer(Target::Raster(1)),
                Key::Layer(Target::Raster(3)),
                Key::Layer(Target::Vector(vector))
            ]
            .into_iter()
            .collect()
        );
        let s = Settings {
            kinds: Kinds::Some([K::Ruler].into_iter().collect()),
            ..Default::default()
        };
        assert_eq!(query(&rows, &s, None), [Key::Layer(Target::Folder(1))]);
        let key = Key::Layer(Target::Raster(3));
        let s = Settings {
            scope: Scope::Inside(X::Panel),
            ..Default::default()
        };
        assert_eq!(query(&rows, &s, Some(key)).len(), 2);
        let scene_before = d.snapshot();
        assert!(
            search_edit::prepare(
                &d,
                &Settings {
                    scope: Scope::Outside(X::Panel),
                    ..Default::default()
                },
                &chosen(&[key, Key::Layer(Target::Folder(0))]),
                Edit::Delete
            )
            .is_err()
        );
        assert_eq!(d.snapshot(), scene_before);
        let s = crate::search_move::prepare(&d, Key::Layer(Target::Folder(1)), [3, -2]).unwrap();
        assert_eq!(s.layers[1].extent(16, 16).offset, [3, -2]);
        assert_eq!(s.layers[2].extent(16, 16).offset, [3, -2]);
        assert_eq!(s.layers[0].placement, d.layers[0].placement);
        assert_eq!(s.scene.rulers.guides[0].offset, [3., -2.]);
        d.commit_state(s).unwrap();
        assert!(d.undo());
        assert_eq!(d.snapshot(), scene_before);
        let selected = chosen(&[Key::Layer(Target::Raster(2))]);
        apply(
            &mut d,
            &Settings::default(),
            &selected,
            Edit::Rename("new name".into()),
        )
        .unwrap();
        assert_eq!(
            q::query(
                &c::rows(&d).unwrap(),
                &Settings {
                    name: "new name".into(),
                    ..Default::default()
                },
                None,
                E::Color
            )
            .unwrap(),
            [Key::Layer(Target::Raster(2))]
        );
        let other = page().unwrap();
        assert!(
            q::query(
                &c::rows(&other).unwrap(),
                &Settings {
                    scope: Scope::Outside(X::Panel),
                    ..Default::default()
                },
                Some(key),
                E::Color
            )
            .unwrap()
            .is_empty()
        );
    }
}
