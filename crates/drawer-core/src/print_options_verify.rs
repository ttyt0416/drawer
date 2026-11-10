use super::*;
use crate::{
    document::{Background, CanvasSpec},
    printing::{self, Settings},
    soft_proof::Intent,
};
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("print-options-core");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = Document::create(
            CanvasSpec {
                width: 1000,
                height: 100,
                dpi: 254.,
                background: Background::Transparent,
            },
            B,
        )
        .map_err(|e| e.to_string())?;
        let snapshot = d.snapshot();
        let counts = d.history_counts();
        for ppp in [1., 1.5, 2.] {
            let calibration = Calibration::measure(
                300. * ppp,
                300. * ppp / (96. / 25.4),
                "Measured monitor".into(),
            )?;
            d.view.angle = 37.;
            d.view.flipped = true;
            d.view.pan = [12., -9.];
            apply_view(&mut d, &calibration, ppp)?;
            let a = d
                .view
                .document_to_screen([0., 0.], [1000., 100.], [0.; 2], ppp);
            let b = d
                .view
                .document_to_screen([1000., 0.], [1000., 100.], [0.; 2], ppp);
            let length = ((b[0] - a[0]).hypot(b[1] - a[1]) * ppp) / calibration.pixels_per_mm;
            if (length - 100.).abs() > 1e-6
                || d.snapshot() != snapshot
                || d.history_counts() != counts
            {
                return Err("Physical100mm view / DPI scale / source isolation".into());
            }
        }
        for (px, mm) in [(0., 1.), (1., 0.), (f64::NAN, 1.), (300., -2.)] {
            if Calibration::measure(px, mm, String::new()).is_ok() {
                return Err("Invalid physical measurement accepted".into());
            }
        }
        let c = AtomicBool::new(false);
        let image = LocalImage {
            width: 2,
            height: 1,
            pixels: vec![[20, 90, 180, 128], [0, 0, 0, 0]],
        };
        for (channel, expected) in [
            (Channel::Red, [20, 20, 20, 128]),
            (Channel::Green, [90, 90, 90, 128]),
            (Channel::Blue, [180, 180, 180, 128]),
            (Channel::Alpha, [127, 127, 127, 255]),
        ] {
            let out = mapped(
                &image,
                &Options {
                    channel,
                    ..Default::default()
                },
                false,
                false,
                &c,
            )?;
            if out.pixels[0] != expected || image.pixels[0] != [20, 90, 180, 128] {
                return Err("Selected print channel/alpha/source".into());
            }
        }
        let mut draft = crate::layer_roles_verify::page()?;
        draft.layers[0].pixels.fill([240, 30, 10, 255]);
        let state = crate::layer_roles::prepare(
            &draft,
            crate::layer_search::Key::Layer(properties::Target::Raster(1)),
            crate::layer_roles::Flags {
                draft: true,
                ..Default::default()
            },
            None,
        )?;
        draft.commit_state(state)?;
        let before = draft.snapshot();
        let history = draft.history_counts();
        let hidden = compose(&draft, &Options::default(), 300., B, &c)?;
        let shown = compose(
            &draft,
            &Options {
                include_drafts: true,
                ..Default::default()
            },
            300.,
            B,
            &c,
        )?;
        if hidden == shown || draft.snapshot() != before || draft.history_counts() != history {
            return Err("Draft print-only inclusion/source".into());
        }
        let text = crate::text_version_verify::fixture()?;
        let with = compose(&text, &Options::default(), text.dpi, B, &c)?;
        let without = compose(
            &text,
            &Options {
                include_text: false,
                ..Default::default()
            },
            text.dpi,
            B,
            &c,
        )?;
        if with == without {
            return Err("Text exclusion did not change editable page output".into());
        }
        let mut guides = crate::layer_roles_verify::page()?;
        guides.scene.comics.page.trim_mm = [0.5, 0.5];
        guides.scene.comics.page.bleed_mm = 0.02;
        guides.scene.comics.page.safe_mm = 0.02;
        let without = compose(&guides, &Options::default(), 300., B, &c)?;
        let with = compose(
            &guides,
            &Options {
                crop_guides: true,
                ..Default::default()
            },
            300.,
            B,
            &c,
        )?;
        if with == without {
            return Err("Page trim/bleed guide output option".into());
        }
        let source = printing::verify::fixture();
        let mut s = Settings {
            fit: false,
            ..Default::default()
        };
        let base = printing::virtual_pages(&source[..1], &s, 96., 64 * 1024 * 1024, &c)?;
        s.output.registration = true;
        let marked = printing::virtual_pages(&source[..1], &s, 96., 64 * 1024 * 1024, &c)?;
        if base == marked || !marked[0].image.pixels.contains(&[0, 0, 0, 255]) {
            return Err("Paper registration marks not applied".into());
        }
        if printing::virtual_pages(&source, &s, 96., 1, &c).is_ok()
            || printing::virtual_pages(&source, &s, 96., B, &AtomicBool::new(true)).is_ok()
        {
            return Err("Print budget/cancellation ignored".into());
        }
        let profile = dir.join("output.icc");
        let bytes = crate::export_color::profile_bytes(&crate::color_profile::RgbProfile::Srgb)?;
        std::fs::write(&profile, &bytes).map_err(|e| e.to_string())?;
        let mut options = Options {
            profile: Some(profile.clone()),
            ..Default::default()
        };
        for (intent, code) in [
            (Intent::Saturation, 1),
            (Intent::Relative, 2),
            (Intent::Perceptual, 4),
            (Intent::Absolute, 8),
        ] {
            options.intent = intent;
            let out = mapped(&image, &options, true, true, &c)?;
            if gdi_intent(intent) != code
                || out.pixels[0][3] != 128
                || out.pixels[0][..3]
                    .iter()
                    .zip([20, 90, 180])
                    .any(|(a, b)| (*a as i16 - b).abs() > 2)
            {
                return Err("RGB ICC proof/intent/alpha".into());
            }
        }
        let frozen = FrozenProfile::capture(&dir, &options, true)?.ok_or("Missing frozen ICC")?;
        let path = frozen.path().to_owned();
        std::fs::write(&profile, b"changed source ICC").map_err(|e| e.to_string())?;
        if std::fs::read(&path).map_err(|e| e.to_string())? != bytes
            || frozen.sha256 != crate::asset_catalog::hash(&bytes)
        {
            return Err("Output ICC did not freeze reviewed bytes".into());
        }
        #[cfg(windows)]
        if std::fs::write(&path, b"overwrite").is_ok() {
            return Err("Frozen ICC was writable while printing".into());
        }
        drop(frozen);
        if path.exists() {
            return Err("ICC temporary copy leaked".into());
        }
        options.profile = Some(dir.join("missing.icc"));
        if mapped(&image, &options, true, true, &c).is_ok()
            || mapped(&image, &options, false, true, &c)?.pixels != image.pixels
        {
            return Err("Inactive ICC option / missing active ICC".into());
        }
        let storage =
            crate::storage::UserStorage::at(dir.join("data")).map_err(|e| e.to_string())?;
        s.output.calibration = Some(Calibration::measure(300., 79.375, "Display".into())?);
        s.output.intent = Intent::Absolute;
        s.output.channel = Channel::Alpha;
        s.save(&storage)?;
        if Settings::load(&storage)? != s {
            return Err("Extended print preset restart".into());
        }
        Ok(())
    })();
    crate::asset_catalog::cleanup_verification(root, &dir)?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn physical_calibration_channels_content_icc_marks_cancel_and_presets() {
        let t = crate::test_support::TempDir::new();
        super::run(&t.0).unwrap();
    }
}
