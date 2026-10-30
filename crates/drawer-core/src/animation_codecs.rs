//! Bounded local GIF/APNG and lossless animated WebP containers.
use crate::{
    animation::Fps,
    animation_export::{self as export, Format, Snapshot},
};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
/// Difference of rounded cumulative boundaries prevents per-frame rounding drift.
pub fn duration(fps: Fps, index: u32, units: u32) -> Result<u32, String> {
    let boundary = |n: u32| {
        ((n as u128 * fps.denominator as u128 * units as u128 + fps.numerator as u128 / 2)
            / fps.numerator as u128) as u64
    };
    let ticks = boundary(index + 1) - boundary(index);
    let max = if units == 100 { 65535 } else { 0xffffff };
    if ticks == 0 || ticks > max {
        return Err(
            "Frame timing exceeds the selected format; use PNG sequence + rational FPS".into(),
        );
    }
    Ok(ticks as u32)
}
fn chunk(w: &mut impl Write, tag: &[u8; 4], data: &[u8]) -> Result<(), String> {
    let n = u32::try_from(data.len()).map_err(|_| "WebP chunk exceeds4GiB")?;
    w.write_all(tag)
        .and_then(|_| w.write_all(&n.to_le_bytes()))
        .and_then(|_| w.write_all(data))
        .map_err(|e| e.to_string())?;
    if n % 2 != 0 {
        w.write_all(&[0]).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn u24(b: &mut Vec<u8>, n: u32) {
    b.extend_from_slice(&n.to_le_bytes()[..3]);
}
pub(crate) fn encode(
    s: &Snapshot,
    frames: &[PathBuf],
    out: &Path,
    metadata: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if s.options.format == Format::Movie {
        return crate::movie_export::encode(s, frames, out, cancel);
    }
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(out)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    match s.options.format {
        Format::Apng => {
            let (_, info) = export::png_data(&frames[0], s.layout.pixels() * 32)?;
            let mut e = png::Encoder::with_info(&mut writer, info).map_err(|e| e.to_string())?;
            e.set_animated(s.count(), s.options.plays)
                .map_err(|e| e.to_string())?;
            let (n, d) = export::delay(s.fps())?;
            e.set_frame_delay(n, d).map_err(|e| e.to_string())?;
            e.set_blend_op(png::BlendOp::Source)
                .map_err(|e| e.to_string())?;
            e.set_dispose_op(png::DisposeOp::None)
                .map_err(|e| e.to_string())?;
            e.add_text_chunk("Drawer Export".into(), metadata.into())
                .map_err(|e| e.to_string())?;
            let mut e = e.write_header().map_err(|e| e.to_string())?;
            for f in frames {
                crate::book_io::check_cancel(cancel)?;
                let (b, _) = export::png_data(f, s.layout.pixels() * 32)?;
                e.write_image_data(&b).map_err(|e| e.to_string())?;
            }
            e.finish().map_err(|e| e.to_string())?;
        }
        Format::Gif => {
            let mut e = gif::Encoder::new(
                &mut writer,
                s.layout.width as u16,
                s.layout.height as u16,
                &[],
            )
            .map_err(|e| e.to_string())?;
            if s.options.plays != 1 {
                e.set_repeat(if s.options.plays == 0 {
                    gif::Repeat::Infinite
                } else {
                    gif::Repeat::Finite((s.options.plays - 1) as u16)
                })
                .map_err(|e| e.to_string())?;
            }
            for (n, f) in frames.iter().enumerate() {
                crate::book_io::check_cancel(cancel)?;
                let (mut b, _) = export::png_data(f, s.layout.pixels() * 32)?;
                for p in b.as_chunks_mut::<4>().0 {
                    p[3] = if p[3] < 128 { 0 } else { 255 };
                }
                let mut f = gif::Frame::from_rgba_speed(
                    s.layout.width as u16,
                    s.layout.height as u16,
                    &mut b,
                    10,
                );
                f.delay = duration(s.fps(), n as u32, 100)? as u16;
                f.dispose = gif::DisposalMethod::Background;
                e.write_frame(&f).map_err(|e| e.to_string())?;
            }
            e.into_inner().map_err(|e| e.to_string())?;
        }
        Format::Webp => {
            writer
                .write_all(b"RIFF\0\0\0\0WEBP")
                .map_err(|e| e.to_string())?;
            let icc = crate::animation_render::icc(s)?;
            let mut header = vec![0x12 | if icc.is_some() { 0x20 } else { 0 }, 0, 0, 0];
            u24(&mut header, s.layout.width - 1);
            u24(&mut header, s.layout.height - 1);
            chunk(&mut writer, b"VP8X", &header)?;
            if let Some(icc) = icc {
                chunk(&mut writer, b"ICCP", &icc)?;
            }
            let mut anim = vec![0; 4];
            anim.extend_from_slice(&(s.options.plays as u16).to_le_bytes());
            chunk(&mut writer, b"ANIM", &anim)?;
            for (n, path) in frames.iter().enumerate() {
                crate::book_io::check_cancel(cancel)?;
                let (b, _) = export::png_data(path, s.layout.pixels() * 32)?;
                let mut still = Vec::new();
                image_webp::WebPEncoder::new(&mut still)
                    .encode(
                        &b,
                        s.layout.width,
                        s.layout.height,
                        image_webp::ColorType::Rgba8,
                    )
                    .map_err(|e| e.to_string())?;
                if still.get(12..16) != Some(b"VP8L") {
                    return Err("Unexpected lossless WebP encoder container".into());
                }
                let mut data = vec![0; 6];
                u24(&mut data, s.layout.width - 1);
                u24(&mut data, s.layout.height - 1);
                u24(&mut data, duration(s.fps(), n as u32, 1000)?);
                data.push(2); // no blend, no disposal
                data.extend_from_slice(&still[12..]);
                chunk(&mut writer, b"ANMF", &data)?;
                if writer.stream_position().map_err(|e| e.to_string())? > 2 * 1024 * 1024 * 1024 {
                    return Err("Animated WebP exceeds2GiB".into());
                }
            }
            let len = writer.stream_position().map_err(|e| e.to_string())?;
            writer
                .seek(SeekFrom::Start(4))
                .and_then(|_| writer.write_all(&((len - 8) as u32).to_le_bytes()))
                .map_err(|e| e.to_string())?;
        }
        _ => return Err("Choose an animated image format".into()),
    }
    writer
        .flush()
        .and_then(|_| writer.get_ref().sync_all())
        .map_err(|e| e.to_string())?;
    drop(writer);
    validate(s, frames, out, cancel)
}
fn validate(
    s: &Snapshot,
    frames: &[PathBuf],
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    match s.options.format {
        Format::Webp => {
            let mut d = image_webp::WebPDecoder::new(BufReader::new(
                File::open(path).map_err(|e| e.to_string())?,
            ))
            .map_err(|e| e.to_string())?;
            d.set_memory_limit(s.layout.pixels() * 32);
            if d.dimensions() != (s.layout.width, s.layout.height)
                || d.num_frames() != s.count()
                || !d.is_animated()
            {
                return Err("WebP dimensions/frame count verification".into());
            }
            for (n, path) in frames.iter().enumerate() {
                crate::book_io::check_cancel(cancel)?;
                let mut b = vec![0; s.layout.pixels() * 4];
                let t = d.read_frame(&mut b).map_err(|e| e.to_string())?;
                let expected = export::png_data(path, s.layout.pixels() * 32)?.0;
                // Fully transparent RGB is semantically undefined in WebP; compare visible channels.
                if t != duration(s.fps(), n as u32, 1000)?
                    || b.as_chunks::<4>()
                        .0
                        .iter()
                        .zip(expected.as_chunks::<4>().0.iter())
                        .any(|(a, b)| a[3] != b[3] || a[3] > 0 && a[..3] != b[..3])
                {
                    return Err("WebP decoded pixels/timing verification".into());
                }
            }
        }
        Format::Apng => {
            let mut d =
                png::Decoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?))
                    .read_info()
                    .map_err(|e| e.to_string())?;
            if !d
                .info()
                .animation_control
                .is_some_and(|c| c.num_frames == s.count() && c.num_plays == s.options.plays)
            {
                return Err("APNG count/repeat verification".into());
            }
            for path in frames {
                crate::book_io::check_cancel(cancel)?;
                let mut b = vec![0; s.layout.pixels() * 4];
                let info = d.next_frame(&mut b).map_err(|e| e.to_string())?;
                if info.width != s.layout.width
                    || info.height != s.layout.height
                    || b != export::png_data(path, s.layout.pixels() * 32)?.0
                {
                    return Err("APNG decoded frame verification".into());
                }
            }
        }
        Format::Gif => {
            let mut opts = gif::DecodeOptions::new();
            opts.set_color_output(gif::ColorOutput::RGBA);
            let mut d = opts
                .read_info(BufReader::new(File::open(path).map_err(|e| e.to_string())?))
                .map_err(|e| e.to_string())?;
            if d.width() != s.layout.width as u16 || d.height() != s.layout.height as u16 {
                return Err("GIF size verification".into());
            }
            let mut n = 0;
            while let Some(f) = d.read_next_frame().map_err(|e| e.to_string())? {
                crate::book_io::check_cancel(cancel)?;
                if f.delay as u32 != duration(s.fps(), n, 100)?
                    || f.width != s.layout.width as u16
                    || f.height != s.layout.height as u16
                    || f.buffer.len() != s.layout.pixels() * 4
                    || f.buffer
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[3] != 0 && p[3] != 255)
                {
                    return Err("GIF frame / timing / alpha verification".into());
                }
                n += 1;
            }
            if n != s.count() {
                return Err("GIF frame count verification".into());
            }
        }
        _ => {}
    }
    Ok(())
}
