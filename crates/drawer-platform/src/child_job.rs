//! OS-owned helper lifetime: closing/exiting the host kills only its acquisition/inference
//! helper job. The WIA service and other applications are never assigned.
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        },
        Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
    },
};
pub struct ChildJob(isize);
impl Drop for ChildJob {
    fn drop(&mut self) {
        // SAFETY: unique owned kernel job handle; thread-independent, closed once.
        let _ = unsafe { CloseHandle(HANDLE(self.0 as *mut _)) };
    }
}
pub fn acquisition_child_job(child: &std::process::Child) -> Result<ChildJob, String> {
    // SAFETY: unnamed OS job, no inherited/shared object name or security buffer.
    let handle = unsafe { CreateJobObjectW(None, None) }.map_err(|e| e.to_string())?;
    let job = ChildJob(handle.0 as isize);
    let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation:
            windows::Win32::System::JobObjects::JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY,
                ..Default::default()
            },
        ProcessMemoryLimit: 512 * 1024 * 1024,
        ..Default::default()
    };
    // SAFETY: complete initialized ABI structure, correct class and exact size.
    unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    }
    .map_err(|e| e.to_string())?;
    // SAFETY: the caller retains this spawned Child and its process handle, so
    // its PID cannot be reused. Only rights needed for job assignment requested.
    let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, child.id()) }
        .map_err(|e| e.to_string())?;
    // SAFETY: our fresh job and this owned child process; no other PID accepted.
    let assigned = unsafe { AssignProcessToJobObject(handle, process) };
    // SAFETY: unique OpenProcess reference, closed once regardless of assignment.
    let _ = unsafe { CloseHandle(process) };
    assigned.map_err(|e| format!("Could not contain local acquisition helper: {e}"))?;
    Ok(job)
}
