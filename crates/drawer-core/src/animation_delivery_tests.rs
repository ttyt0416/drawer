use super::*;
use crate::{
    animation_render::{Area, View},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document, PlacedRaster},
    scene::Rect,
};
fn options() -> Options {
    Options {
        last: 9,
        ..Default::default()
    }
}
fn simple() -> Document {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 4,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap();
    d.commit_state(crate::timeline_editor::create(&d, 10, Default::default()).unwrap())
        .unwrap();
    d
}
fn render(d: &Document, o: Options) -> (Snapshot, Vec<u8>) {
    let s = prepare(d, o, B).unwrap();
    let b = crate::animation_render::frame(&s, o.first, &AtomicBool::new(false)).unwrap();
    (s, b)
}
#[test]
fn area_union_selection_coverage_overflow_and_camera_switch() {
    let mut d = simple();
    d.layers[0].pixels[5] = [200, 50, 10, 255];
    assert!(
        prepare(
            &d,
            Options {
                view: View {
                    area: Area::Selection,
                    ..Default::default()
                },
                ..options()
            },
            B
        )
        .is_err()
    );
    d.scene.selection_mask = Some(crate::selection::Mask {
        runs: vec![crate::selection::Run {
            x: 1,
            y: 1,
            length: 1,
            alpha: 128,
        }],
    });
    let (s, b) = render(
        &d,
        Options {
            view: View {
                area: Area::Selection,
                ..Default::default()
            },
            ..options()
        },
    );
    assert_eq!(
        (s.layout.origin, s.layout.width, s.layout.height),
        ([1, 1], 1, 1)
    );
    assert_eq!(b, [200, 50, 10, 128]);
    d.scene.animation_output.keys = vec![
        crate::animation_output::Frame {
            frame: 0,
            bounds: Rect {
                min: [1., 1.],
                max: [3., 3.],
            },
            creation: Default::default(),
        },
        crate::animation_output::Frame {
            frame: 5,
            bounds: Rect {
                min: [0., 2.],
                max: [5., 4.],
            },
            creation: Default::default(),
        },
    ];
    let s = prepare(
        &d,
        Options {
            view: View {
                area: Area::OutputFrame,
                ..Default::default()
            },
            ..options()
        },
        B,
    )
    .unwrap();
    assert_eq!(
        (s.layout.origin, s.layout.width, s.layout.height),
        ([0, 1], 5, 3)
    );
    for n in 0..10 {
        assert_eq!(
            crate::animation_render::frame(&s, n, &AtomicBool::new(false))
                .unwrap()
                .len(),
            5 * 3 * 4
        );
    }
    d.scene.selection_mask = None;
    d.layers[0].pixels = vec![[255, 0, 0, 255]; 4];
    d.layers[0].placement = Some(PlacedRaster {
        width: 2,
        height: 2,
        offset: [-2, -1],
    });
    let (s, b) = render(
        &d,
        Options {
            view: View {
                area: Area::Overflow,
                ..Default::default()
            },
            ..options()
        },
    );
    assert_eq!(
        (s.layout.origin, s.layout.width, s.layout.height),
        ([-2, -1], 6, 5)
    );
    assert_eq!(&b[..4], &[255, 0, 0, 255]);
    d.commit_state(crate::camera2d::enable(&d, true).unwrap())
        .unwrap();
    d.commit_state(
        crate::keyframes::prepare(
            &d,
            crate::keyframes::Owner::Camera,
            crate::keyframes::Edit::Put {
                key: crate::keyframes::Keyframe {
                    frame: 0,
                    pose: crate::keyframes::Pose {
                        offset: [-2., -1.],
                        ..Default::default()
                    },
                    curve: Default::default(),
                },
                replace: true,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let (_, on) = render(&d, options());
    let (_, off) = render(
        &d,
        Options {
            view: View {
                camera: false,
                ..Default::default()
            },
            ..options()
        },
    );
    assert_eq!(&on[..4], &[255, 0, 0, 255]);
    assert_eq!(&off[..4], &[0, 0, 0, 0]);
}
#[test]
fn drafts_nested_cels_paper_frame_lines_live_shadows_and_icc() {
    let mut d = simple();
    d.layers[0].pixels[5] = [100, 30, 20, 128];
    let key = crate::layer_search::Key::Layer(crate::layer_properties::raster(&d, 0));
    d.commit_state(
        crate::layer_roles::prepare(
            &d,
            key,
            crate::layer_roles::Flags {
                draft: true,
                reference: false,
            },
            None,
        )
        .unwrap(),
    )
    .unwrap();
    let (_, off) = render(&d, options());
    let (_, on) = render(
        &d,
        Options {
            view: View {
                drafts: true,
                ..Default::default()
            },
            ..options()
        },
    );
    assert_eq!(off[5 * 4 + 3], 0);
    assert_eq!(on[5 * 4 + 3], 128);
    let mut nested = crate::animation_export_verify::fixture();
    let id = nested.scene.cels.definitions[0].object;
    let mut child = nested.scene.cels.get(id).unwrap().open().unwrap();
    let key = crate::layer_search::Key::Layer(crate::layer_properties::raster(&child, 0));
    child
        .commit_state(
            crate::layer_roles::prepare(
                &child,
                key,
                crate::layer_roles::Flags {
                    draft: true,
                    reference: false,
                },
                None,
            )
            .unwrap(),
        )
        .unwrap();
    nested
        .commit_state(crate::cels::replace(&nested, id, &child, B).unwrap())
        .unwrap();
    let (_, off) = render(&nested, Options::default());
    let (_, on) = render(
        &nested,
        Options {
            view: View {
                drafts: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(off[3], 0);
    assert_eq!(on[3], 128);
    let mut d = simple();
    d.commit_state(crate::paper_background::prepare(&d, [20, 60, 90], true).unwrap())
        .unwrap();
    let (_, transparent) = render(&d, options());
    let (_, opaque) = render(
        &d,
        Options {
            view: View {
                transparent: false,
                ..Default::default()
            },
            ..options()
        },
    );
    assert_eq!(transparent[3], 0);
    assert_eq!(&opaque[..4], &[20, 60, 90, 255]);
    let mut d = simple();
    let frame = crate::comics::defaults(Rect {
        min: [1., 1.],
        max: [3., 3.],
    })[1]
        .clone();
    d.commit_state(crate::comics::prepare(&d, None, frame).unwrap())
        .unwrap();
    let (_, on) = render(&d, options());
    let (_, off) = render(
        &d,
        Options {
            view: View {
                frame_lines: false,
                ..Default::default()
            },
            ..options()
        },
    );
    assert!(on.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    assert!(off.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    let mut d = simple();
    d.layers[0].pixels[5] = [200, 100, 50, 255];
    let id = d.scene.layer_ids.id(0);
    d.scene.surface_effects.rasters.insert(
        id,
        crate::surface_effects::Stack {
            effects: vec![crate::surface_effects::Entry {
                operation: crate::surface_effects::Operation::Shadow {
                    blur: 0,
                    direction: 0.,
                    distance: 1.,
                    color: [0; 3],
                    opacity: 1.,
                },
                enabled: true,
                scope: None,
            }],
            ..Default::default()
        },
    );
    let (_, on) = render(&d, options());
    let (_, off) = render(
        &d,
        Options {
            view: View {
                shadows: false,
                ..Default::default()
            },
            ..options()
        },
    );
    assert_ne!(on, off);
    let d = crate::animation_export_verify::fixture();
    let (s, _) = render(&d, Options::default());
    assert!(crate::animation_render::icc(&s).unwrap().is_some());
    let (s, _) = render(
        &d,
        Options {
            view: View {
                icc: false,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(crate::animation_render::icc(&s).unwrap().is_none());
}
#[test]
fn ten_frame_capture_excludes_later_document_comp_selection_and_watermark_changes() {
    let t = crate::test_support::TempDir::new();
    let mut d = simple();
    d.layers[0].pixels[0] = [200, 30, 20, 255];
    let watermark = t.0.join("watermark.png");
    image::save_buffer_with_format(
        &watermark,
        &[30, 200, 80, 128],
        1,
        1,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .unwrap();
    let o = Options {
        view: View {
            noise: 12,
            seed: 938,
            ..Default::default()
        },
        ..options()
    };
    let s = prepare_with(&d, o, B, Some(&watermark), None).unwrap();
    let hash = s.watermark_hash();
    let cancel = AtomicBool::new(false);
    let progress = AtomicU32::new(0);
    write(&s, &t.0.join("expected"), false, &cancel, &progress).unwrap();
    let comp = crate::layer_comps::prepare(
        &d,
        crate::layer_comps::Command::Capture {
            name: "Before output".into(),
            replace: None,
        },
    )
    .unwrap();
    d.commit_state(comp.state).unwrap();
    let outcome = publish_with(&s, &t.0.join("actual"), false, &cancel, &progress, |n| {
        if n == 2 {
            d.edit_pixel(0, 0, 0, [0, 0, 255, 255]);
            d.current_frame = 7;
            d.scene.selection = Some(Rect {
                min: [2., 2.],
                max: [3., 3.],
            });
            d.scene.layer_comps.comps[0].name = "Changed during output".into();
            image::save_buffer_with_format(
                &watermark,
                &[255, 0, 0, 255],
                1,
                1,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .unwrap();
        }
    });
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(s.watermark_hash(), hash);
    for n in 1..=10 {
        let name = format!("frame-{n:06}.png");
        assert_eq!(
            fs::read(t.0.join("expected").join(&name)).unwrap(),
            fs::read(t.0.join("actual").join(name)).unwrap()
        );
    }
    assert_eq!(d.current_frame, 7);
    assert_eq!(d.layers[0].pixels[0], [0, 0, 255, 255]);
    // Repeated output and a static source have the same coordinate-fixed noise in every frame.
    let first = crate::animation_render::frame(&s, 0, &cancel).unwrap();
    assert_eq!(
        first,
        crate::animation_render::frame(&s, 9, &cancel).unwrap()
    );
    let rec: serde_json::Value =
        serde_json::from_slice(&fs::read(outcome.log.unwrap()).unwrap()).unwrap();
    assert_eq!(rec["snapshot"]["options"]["view"]["seed"], 938);
}
#[test]
fn partial_cancel_restart_late_collision_and_source_protection() {
    let t = crate::test_support::TempDir::new();
    let d = simple();
    let s = prepare(&d, options(), B).unwrap();
    let cancel = AtomicBool::new(false);
    let progress = AtomicU32::new(0);
    let out = t.0.join("partial");
    let result = publish_with(&s, &out, false, &cancel, &progress, |n| {
        if n == 2 {
            cancel.store(true, Ordering::Relaxed)
        }
    });
    assert!(result.error.is_some());
    assert_eq!(result.frames, 3);
    assert_eq!(fs::read_dir(&out).unwrap().count(), 3);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(result.log.unwrap()).unwrap()).unwrap();
    assert_eq!(record["status"], "canceled");
    assert_eq!(record["completed"].as_array().unwrap().len(), 3);
    assert_eq!(record["not_completed"].as_array().unwrap().len(), 7);
    cancel.store(false, Ordering::Relaxed);
    let restarted = publish(&s, &t.0.join("restarted"), false, &cancel, &progress);
    assert!(restarted.error.is_none());
    assert_eq!(restarted.frames, 10);
    assert!(publish(&s, &out, true, &cancel, &progress).error.is_some());
    assert_eq!(fs::read_dir(&out).unwrap().count(), 3);
    let collision = t.0.join("collision");
    let result = publish_with(&s, &collision, false, &cancel, &progress, |n| {
        if n == 0 {
            fs::write(collision.join("frame-000002.png"), b"foreign").unwrap();
        }
    });
    assert_eq!(result.frames, 1);
    assert!(result.error.is_some());
    assert_eq!(
        fs::read(collision.join("frame-000002.png")).unwrap(),
        b"foreign"
    );
    assert!(fs::read_dir(&t.0).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".Drawer-animation")
    }));
    let mut s = prepare(
        &d,
        Options {
            format: Format::Apng,
            ..options()
        },
        B,
    )
    .unwrap();
    let source = t.0.join("original.apng");
    fs::write(&source, b"original source").unwrap();
    s.protected.push(source.clone());
    assert!(write(&s, &source, true, &cancel, &progress).is_err());
    assert_eq!(fs::read(source).unwrap(), b"original source");
}
#[test]
fn local_presets_roundtrip_validate_and_do_not_restore_execution_authorization() {
    let p = Preset {
        version: 1,
        options: Options {
            format: Format::Gif,
            view: View {
                area: Area::Selection,
                seed: 4321,
                noise: 20,
                camera: false,
                ..Default::default()
            },
            ..options()
        },
        watermark: Some(PathBuf::from("C:/local/watermark.png")),
        encoder: None,
    };
    let b = p.encode().unwrap();
    assert_eq!(Preset::decode(&b).unwrap().options, p.options);
    assert!(
        Preset::decode(br#"{"version":99,"options":{},"watermark":null,"encoder":null}"#).is_err()
    );
    assert!(Preset::decode(&vec![0; 65537]).is_err());
    let t = crate::test_support::TempDir::new();
    save_preset(&t.0.join("preset.json"), &b).unwrap();
    assert_eq!(fs::read(t.0.join("preset.json")).unwrap(), b);
    assert!(!String::from_utf8(b).unwrap().contains("overwrite"));
}
#[test]
fn local_movie_self_test_actual_output_alpha_audio() {
    let Some(decoder) = crate::movie::Decoder::discover() else {
        return;
    };
    let cancel = AtomicBool::new(false);
    let v = crate::movie_export::verify(decoder, &cancel).unwrap();
    assert!(v.codecs.contains(&crate::movie_export::Codec::Ffv1));
    assert!(v.codecs.contains(&crate::movie_export::Codec::H264));
    let t = crate::test_support::TempDir::new();
    let mut d = crate::keyframes_verify::fixture();
    d.animation.as_mut().unwrap().audio = vec![crate::animation::Audio {
        name: "Test".into(),
        rate: 48000,
        channels: 2,
        start: 0,
        sample_count: 48000,
        samples: std::sync::Arc::new(vec![100; 48000]),
    }];
    let progress = AtomicU32::new(0);
    for codec in &v.codecs {
        let s = prepare_with(
            &d,
            Options {
                last: 2,
                format: Format::Movie,
                codec: *codec,
                audio: true,
                ..Default::default()
            },
            B,
            None,
            Some(&v),
        )
        .unwrap();
        let p = t.0.join(format!("movie.{}", codec.extension()));
        write(&s, &p, false, &cancel, &progress).unwrap();
        assert!(p.is_file());
        let samples = crate::audio_mix::boundary(3, s.fps(), 48000) as usize;
        let sound = crate::movie::run(
            std::process::Command::new(&v.decoder.ffmpeg)
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&p)
                .args([
                    "-map", "0:a:0", "-f", "s16le", "-ac", "2", "-ar", "48000", "pipe:1",
                ]),
            (samples + 2048) * 4,
            &cancel,
        )
        .unwrap();
        if *codec == crate::movie_export::Codec::Ffv1 {
            assert_eq!(sound.len(), samples * 4);
            assert!(
                sound
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|p| i16::from_le_bytes(*p) == 100)
            );
        } else {
            assert!((samples * 4..=(samples + 2048) * 4).contains(&sound.len()));
        }
    }
    assert!(crate::movie_export::verify(v.decoder.clone(), &AtomicBool::new(true)).is_err());
}
