use super::*;
use std::{
    io::Write,
    process::{Child, Command, Stdio},
    sync::atomic::AtomicBool,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new(base: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
        let base = base.canonicalize().map_err(|e| e.to_string())?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = base.join(format!("drawer-infer-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("drawer-infer-"))
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// The helper waits for stdin before parsing any model. The host assigns the
/// private 512 MiB Windows job before releasing that gate, and kills/reaps on
/// cancellation, timeout, failure, host exit or a dropped UI worker.
pub fn execute(
    base: &Path,
    executable: &Path,
    request: &Request,
    input: &LocalImage,
    hints: Option<&LocalImage>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Output, String> {
    core::checked(cancel)?;
    let scratch = Scratch::new(base)?;
    std::fs::write(
        scratch.0.join("request.json"),
        serde_json::to_vec(request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    core::write_image(&scratch.0.join("input.rgba"), input, core::MAX_INPUT)?;
    if let Some(h) = hints {
        core::write_image(&scratch.0.join("hints.rgba"), h, core::MAX_INPUT)?;
    }
    core::checked(cancel)?;
    let mut command = Command::new(executable);
    command
        .arg("--local-infer-helper")
        .arg(&scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut process = Process(command.spawn().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    let _job = drawer_platform::local_inference_child_job(&process.0)?;
    process
        .0
        .stdin
        .take()
        .ok_or("Inference gate unavailable")?
        .write_all(&[1])
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        core::checked(cancel)?;
        if start.elapsed() > timeout {
            return Err("Local inference timed out; helper closed".into());
        }
        if let Some(status) = process.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("Local inference helper stopped (possibly insufficient memory / 512 MiB limit); original unchanged".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    core::checked(cancel)?;
    let reply: Reply = serde_json::from_slice(&read_bytes(&scratch.0.join("reply.json"), 16384)?)
        .map_err(|e| e.to_string())?;
    if let Some(e) = reply.error {
        return Err(e);
    }
    let output = Output {
        task: request.task,
        report: reply.report,
        points: reply.points,
        image: if request.task.joints() == 0 {
            Some(core::read_image(
                &scratch.0.join("output.rgba"),
                core::MAX_OUTPUT,
            )?)
        } else {
            None
        },
    };
    output.validate()?;
    Ok(output)
}
