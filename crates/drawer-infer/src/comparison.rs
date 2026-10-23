//! Fixed, independently drawn diagnostic samples. Metrics describe this small
//! synthetic set, not natural-image accuracy or artistic preference.
use super::*;
fn joints(task: Task) -> Vec<[f32; 3]> {
    if task == Task::Body {
        vec![
            [0.5, 0.13, 1.],
            [0.47, 0.11, 1.],
            [0.53, 0.11, 1.],
            [0.43, 0.14, 1.],
            [0.57, 0.14, 1.],
            [0.35, 0.3, 1.],
            [0.65, 0.3, 1.],
            [0.25, 0.48, 1.],
            [0.75, 0.48, 1.],
            [0.18, 0.62, 1.],
            [0.82, 0.62, 1.],
            [0.42, 0.56, 1.],
            [0.58, 0.56, 1.],
            [0.35, 0.75, 1.],
            [0.65, 0.75, 1.],
            [0.3, 0.94, 1.],
            [0.7, 0.94, 1.],
        ]
    } else {
        let mut p = vec![[0.5, 0.9, 1.]];
        for f in 0..5 {
            for k in 0..4 {
                p.push([
                    0.16 + f as f32 * 0.16,
                    0.68 - k as f32 * (0.12 + 0.01 * f as f32),
                    1.,
                ]);
            }
        }
        p
    }
}
fn truth(task: Task) -> LocalImage {
    let mut image = LocalImage {
        width: 64,
        height: 64,
        pixels: vec![[245, 245, 245, 255]; 4096],
    };
    if task.joints() > 0 {
        let points = joints(task);
        for [a, b] in task.edges() {
            let p = points[a];
            let q = points[b];
            for k in 0..128 {
                let t = k as f32 / 127.;
                let x = ((p[0] * (1. - t) + q[0] * t) * 63.).round() as i32;
                let y = ((p[1] * (1. - t) + q[1] * t) * 63.).round() as i32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (xx, yy) = (x + dx, y + dy);
                        if (0..64).contains(&xx) && (0..64).contains(&yy) {
                            image.pixels[(yy * 64 + xx) as usize] = [45, 60, 75, 255];
                        }
                    }
                }
            }
        }
    } else {
        for y in 0..64 {
            for x in 0..64 {
                let i = y * 64 + x;
                let dx = x as i32 - 31;
                let dy = y as i32 - 31;
                image.pixels[i] = if dx * dx + dy * dy < 22 * 22 {
                    if x < 32 {
                        [220, 80, 50, 255]
                    } else {
                        [50, 110, 220, 255]
                    }
                } else {
                    [245, 245, 245, 255]
                };
            }
        }
    }
    image
}
pub fn input(task: Task) -> (LocalImage, Option<LocalImage>) {
    let mut image = truth(task);
    let mut hints = None;
    if task == Task::HintColor {
        let mut h = LocalImage {
            width: 64,
            height: 64,
            pixels: vec![[0; 4]; 4096],
        };
        for y in 27..36 {
            for x in [24, 40] {
                h.pixels[y * 64 + x] = image.pixels[y * 64 + x];
            }
        }
        for p in &mut image.pixels {
            let l =
                (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).round() as u8;
            *p = [l, l, l, 255];
        }
        hints = Some(h);
    } else if task == Task::Denoise {
        let mut seed = 0x167u32;
        for p in &mut image.pixels {
            for v in &mut p[..3] {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let delta = ((seed >> 24) % 49) as i32 - 24;
                *v = (*v as i32 + delta).clamp(0, 255) as u8;
            }
        }
    } else if task == Task::Upscale {
        let pixels = (0..32)
            .flat_map(|y| (0..32).map(move |x| (x, y)))
            .map(|(x, y)| {
                let mut p = [0u8; 4];
                for (c, v) in p.iter_mut().enumerate() {
                    let sum: u16 = (0..2)
                        .flat_map(|dy| (0..2).map(move |dx| (dx, dy)))
                        .map(|(dx, dy)| image.pixels[(y * 2 + dy) * 64 + x * 2 + dx][c] as u16)
                        .sum();
                    *v = ((sum + 2) / 4) as u8;
                }
                p
            })
            .collect();
        image = LocalImage {
            width: 32,
            height: 32,
            pixels,
        };
    }
    (image, hints)
}
fn mae(image: &LocalImage, target: &LocalImage) -> f64 {
    let mut sum = 0.;
    for y in 0..64 {
        for x in 0..64 {
            let p = sample(
                image,
                (x as f64 + 0.5) * image.width as f64 / 64. - 0.5,
                (y as f64 + 0.5) * image.height as f64 / 64. - 0.5,
            );
            let q = target.pixels[y * 64 + x];
            for c in 0..3 {
                sum += (p[c] as f64 - q[c] as f64 / 255.).abs();
            }
        }
    }
    sum / (4096. * 3.)
}
pub fn report(task: Task, output: &Output) -> Result<String, String> {
    output.validate()?;
    if output.task != task {
        return Err("Comparison task mismatch".into());
    }
    if task.joints() > 0 {
        let expected = joints(task);
        let error = output
            .points
            .iter()
            .zip(expected)
            .map(|(a, b)| (((a[0] - b[0]) * 64.).powi(2) + ((a[1] - b[1]) * 64.).powi(2)) as f64)
            .sum::<f64>();
        Ok(format!(
            "Fixed comparison set v1 / {}: joint RMSE {:.3} pixels on the 64px synthetic skeleton. All joints included, confidence shown separately. This is a diagnostic sample, not measured natural-image accuracy. Comparison results cannot be adopted.",
            task.label(),
            (error / task.joints() as f64).sqrt()
        ))
    } else {
        let target = truth(task);
        let (baseline, _) = input(task);
        let actual = mae(output.image.as_ref().unwrap(), &target);
        let before = mae(&baseline, &target);
        Ok(format!(
            "Fixed comparison set v1 / {}: RGB MAE {:.6}, input/bilinear baseline {:.6} (lower is closer to the fixed 64px target). {} → {} pixels. Output aligned to the 64px reference for comparison. Synthetic shapes/colors only; this is not a quality certification. Comparison results cannot be adopted.",
            task.label(),
            actual,
            before,
            baseline.width,
            output.image.as_ref().unwrap().width
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_samples_are_repeatable_and_metrics_have_independent_zero_and_nonzero_controls() {
        for task in Task::ALL {
            let (a, h) = input(task);
            assert_eq!((a.clone(), h.clone()), input(task));
            assert!(core::valid_image(&a, core::MAX_INPUT));
            if task == Task::HintColor {
                assert!(h.unwrap().pixels.iter().any(|p| p[3] > 0));
            }
            let ideal = Output {
                task,
                image: (task.joints() == 0).then(|| truth(task)),
                points: if task.joints() > 0 {
                    joints(task)
                } else {
                    vec![]
                },
                report: String::new(),
            };
            let text = report(task, &ideal).unwrap();
            assert!(
                text.contains(if task.joints() > 0 {
                    "RMSE 0.000"
                } else {
                    "MAE 0.000000"
                }),
                "{text}"
            );
            if task.joints() == 0 {
                let wrong = Output {
                    task,
                    image: Some(LocalImage {
                        width: 64,
                        height: 64,
                        pixels: vec![[0, 0, 0, 255]; 4096],
                    }),
                    points: vec![],
                    report: String::new(),
                };
                assert!(!report(task, &wrong).unwrap().contains("MAE 0.000000"));
            }
        }
    }
}
