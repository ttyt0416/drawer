//! Capability-free pixel bytecode. This interpreter has no host imports, handles,
//! filesystem, process, network, native code, stack, or plugin-controlled allocator.
use crate::references::LocalImage;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_PIXELS: usize = 1024 * 1024;
pub const MAX_FUEL: u64 = 2 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub version: u32,
    pub name: String,
    pub instructions: Vec<Instruction>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum Instruction {
    RequireSize { width: u32, height: u32 },
    Set { index: u32, rgba: [u8; 4] },
    CopyInput { from: u32, to: u32 },
    Invert,
    Jump { instruction: u32 },
    Abort,
    End,
}
pub fn parse(bytes: &[u8]) -> Result<Program, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Plugin exceeds256KiB".into());
    }
    let p: Program =
        serde_json::from_slice(bytes).map_err(|e| format!("Restricted plugin format: {e}"))?;
    if p.version != 1
        || p.name.trim().is_empty()
        || p.name.len() > 128
        || p.name.chars().any(char::is_control)
        || p.instructions.is_empty()
        || p.instructions.len() > 4096
    {
        return Err("Unsupported plugin version, name or instruction count (1..4096)".into());
    }
    Ok(p)
}
pub fn run(
    bytes: &[u8],
    input: &LocalImage,
    budget: usize,
    fuel: u64,
    cancel: &AtomicBool,
) -> Result<LocalImage, String> {
    let p = parse(bytes)?;
    let count = (input.width as usize)
        .checked_mul(input.height as usize)
        .ok_or("Plugin image overflow")?;
    if count == 0
        || count > MAX_PIXELS
        || count != input.pixels.len()
        || count
            .checked_mul(8)
            .and_then(|n| n.checked_add(MAX_BYTES * 4))
            .is_none_or(|n| n > budget)
    {
        return Err(
            "Plugin memory/input limit: at most1,048,576 pixels within the workspace budget".into(),
        );
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Plugin cancelled".into());
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| "Plugin allocation refused")?;
    output.extend_from_slice(&input.pixels);
    let mut fuel = fuel.min(MAX_FUEL);
    let mut pc = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Plugin cancelled".into());
        }
        let op = p
            .instructions
            .get(pc)
            .ok_or("Plugin reached end without End")?;
        let cost = if matches!(op, Instruction::Invert) {
            count as u64
        } else {
            1
        };
        fuel = fuel
            .checked_sub(cost)
            .ok_or("Plugin execution quota exhausted; result discarded")?;
        match *op {
            Instruction::RequireSize { width, height } => {
                if input.width != width || input.height != height {
                    return Err("Plugin document dimensions do not match".into());
                }
            }
            Instruction::Set { index, mut rgba } => {
                if rgba[3] == 0 {
                    rgba = [0; 4];
                }
                *output
                    .get_mut(index as usize)
                    .ok_or("Plugin pixel outside image")? = rgba;
            }
            Instruction::CopyInput { from, to } => {
                let pixel = *input
                    .pixels
                    .get(from as usize)
                    .ok_or("Plugin read outside image")?;
                *output
                    .get_mut(to as usize)
                    .ok_or("Plugin write outside image")? = pixel;
            }
            Instruction::Invert => {
                for (i, pixel) in output.iter_mut().enumerate() {
                    if i % 1024 == 0 && cancel.load(Ordering::Relaxed) {
                        return Err("Plugin cancelled".into());
                    }
                    if pixel[3] != 0 {
                        for channel in &mut pixel[..3] {
                            *channel = 255 - *channel;
                        }
                    }
                }
            }
            Instruction::Jump { instruction } => {
                pc = instruction as usize;
                continue;
            }
            Instruction::Abort => return Err("Plugin reported a failure; result discarded".into()),
            Instruction::End => break,
        }
        pc += 1;
    }
    Ok(LocalImage {
        width: input.width,
        height: input.height,
        pixels: output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> LocalImage {
        LocalImage {
            width: 2,
            height: 1,
            pixels: vec![[10, 20, 30, 255], [0; 4]],
        }
    }
    fn program(ops: &str) -> Vec<u8> {
        format!(r#"{{"version":1,"name":"Owned test","instructions":[{ops}]}}"#).into_bytes()
    }
    #[test]
    fn explicit_pixels_metadata_and_immutable_input() {
        let i = input();
        let before = i.pixels.clone();
        let c = AtomicBool::new(false);
        let b = program(
            r#"{"op":"RequireSize","width":2,"height":1},{"op":"Invert"},{"op":"CopyInput","from":0,"to":1},{"op":"End"}"#,
        );
        let out = run(&b, &i, 2 * 1024 * 1024, 100, &c).unwrap();
        assert_eq!(out.pixels, vec![[245, 235, 225, 255], [10, 20, 30, 255]]);
        assert_eq!(i.pixels, before);
    }
    #[test]
    fn no_imports_native_calls_or_unknown_fields() {
        for op in [
            "network",
            "fetch",
            "process",
            "open",
            "read_file",
            "write_file",
            "panic",
            "Allocate",
            "DLL",
            "WASI",
        ] {
            assert!(parse(&program(&format!(r#"{{"op":"{op}"}}"#))).is_err());
        }
        assert!(
            parse(br#"{"version":1,"name":"x","imports":["fs"],"instructions":[{"op":"End"}]}"#)
                .is_err()
        );
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
    #[test]
    fn loop_memory_bounds_abort_cancel_and_pixel_traps_discard_every_write() {
        let i = input();
        let c = AtomicBool::new(false);
        let before = i.pixels.clone();
        for last in [
            r#"{"op":"Jump","instruction":0}"#,
            r#"{"op":"Jump","instruction":4294967295}"#,
            r#"{"op":"Set","index":4294967295,"rgba":[1,2,3,255]}"#,
            r#"{"op":"Abort"}"#,
        ] {
            let b = program(&format!(
                r#"{{"op":"Set","index":0,"rgba":[1,2,3,255]}},{last}"#
            ));
            assert!(run(&b, &i, 2 * 1024 * 1024, 100, &c).is_err());
            assert_eq!(i.pixels, before);
        }
        let b = program(r#"{"op":"End"}"#);
        assert!(run(&b, &i, 0, 100, &c).is_err());
        c.store(true, Ordering::Relaxed);
        assert!(run(&b, &i, 2 * 1024 * 1024, 100, &c).is_err());
    }
}
