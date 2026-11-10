//! ICC.1:2022 D.6: absolute colorimetry scales media-relative PCS XYZ by media whites.
use moxcms::{
    ColorProfile, Layout, RenderingIntent, ToneReprCurve, TransformF64Executor, TransformOptions,
    Xyzd,
};
use std::sync::Arc;
pub(crate) struct Engine {
    first: Arc<TransformF64Executor>,
    absolute: Option<(Arc<TransformF64Executor>, [f64; 3])>,
    input: usize,
    output: usize,
}
fn channels(layout: Layout) -> Result<usize, String> {
    match layout {
        Layout::Rgb => Ok(3),
        Layout::Rgba => Ok(4),
        Layout::Gray => Ok(1),
        _ => Err("Unsupported ICC channel layout".into()),
    }
}
fn relative_options(intent: RenderingIntent) -> TransformOptions {
    TransformOptions {
        rendering_intent: intent,
        prefer_fixed_point: false,
        allow_extended_range_rgb_xyz: true,
        ..Default::default()
    }
}
fn pcs() -> ColorProfile {
    // Internal linear coordinate carrier: its three RGB numbers ARE PCS XYZ, not a file profile.
    let mut p = ColorProfile::new_srgb();
    p.red_colorant = Xyzd {
        x: 1.,
        y: 0.,
        z: 0.,
    };
    p.green_colorant = Xyzd {
        x: 0.,
        y: 1.,
        z: 0.,
    };
    p.blue_colorant = Xyzd {
        x: 0.,
        y: 0.,
        z: 1.,
    };
    let linear = ToneReprCurve::Parametric(vec![1.]);
    p.red_trc = Some(linear.clone());
    p.green_trc = Some(linear.clone());
    p.blue_trc = Some(linear);
    p.media_white_point = Some(p.white_point);
    p.chromatic_adaptation = None;
    p.cicp = None;
    p
}
fn white(p: &ColorProfile) -> Result<[f64; 3], String> {
    let w = p.media_white_point.unwrap_or(p.white_point);
    let a = [w.x, w.y, w.z];
    if a.iter().any(|v| !v.is_finite() || !(1e-6..=4.).contains(v)) {
        Err("Invalid ICC media white point for absolute colorimetry".into())
    } else {
        Ok(a)
    }
}
impl Engine {
    pub fn new(
        source: &ColorProfile,
        input: Layout,
        target: &ColorProfile,
        output: Layout,
        intent: RenderingIntent,
    ) -> Result<Self, String> {
        let input_channels = channels(input)?;
        let output_channels = channels(output)?;
        if intent != RenderingIntent::AbsoluteColorimetric {
            let first = source
                .create_transform_f64(input, target, output, relative_options(intent))
                .map_err(|e| e.to_string())?;
            return Ok(Self {
                first,
                absolute: None,
                input: input_channels,
                output: output_channels,
            });
        }
        let a = white(source)?;
        let b = white(target)?;
        let scale = [0, 1, 2].map(|n| a[n] / b[n]);
        let carrier = pcs();
        let first = source
            .create_transform_f64(
                input,
                &carrier,
                Layout::Rgb,
                relative_options(RenderingIntent::RelativeColorimetric),
            )
            .map_err(|e| format!("No input-to-PCS absolute ICC direction: {e}"))?;
        let second = carrier
            .create_transform_f64(
                Layout::Rgb,
                target,
                output,
                relative_options(RenderingIntent::RelativeColorimetric),
            )
            .map_err(|e| format!("No PCS-to-output absolute ICC direction: {e}"))?;
        Ok(Self {
            first,
            absolute: Some((second, scale)),
            input: input_channels,
            output: output_channels,
        })
    }
    pub fn transform(&self, input: &[f64], output: &mut [f64]) -> Result<(), String> {
        if !input.len().is_multiple_of(self.input)
            || input.len() / self.input > 1_000_000
            || output.len() != input.len() / self.input * self.output
        {
            return Err("ICC transform channel/size mismatch".into());
        }
        let Some((second, scale)) = &self.absolute else {
            return self
                .first
                .transform(input, output)
                .map_err(|e| e.to_string());
        };
        let count = input.len() / self.input;
        if count == 1 {
            let mut xyz = [0.; 3];
            self.first
                .transform(input, &mut xyz)
                .map_err(|e| e.to_string())?;
            for n in 0..3 {
                xyz[n] *= scale[n];
            }
            return second.transform(&xyz, output).map_err(|e| e.to_string());
        }
        let mut xyz = vec![0.; count * 3];
        self.first
            .transform(input, &mut xyz)
            .map_err(|e| e.to_string())?;
        for p in xyz.as_chunks_mut::<3>().0 {
            for n in 0..3 {
                p[n] *= scale[n];
            }
        }
        second.transform(&xyz, output).map_err(|e| e.to_string())
    }
}
