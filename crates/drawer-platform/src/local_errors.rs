//! Process-local Windows error policy; never writes WER exclusions or sends a report.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetErrorMode() -> u32;
    fn SetErrorMode(mode: u32) -> u32;
}
const NO_FAULT_REPORT: u32 = 0x0002;
const LOCAL_ERRORS: u32 = 0x8003;
pub fn reports_disabled() -> bool {
    // SAFETY: process-local query has no pointer arguments.
    unsafe { GetErrorMode() & NO_FAULT_REPORT != 0 }
}
pub fn configure() -> Result<(), String> {
    // SAFETY: preserve existing flags and suppress critical/open-file UI and the
    // Windows Error Reporting invocation (SEM_NOGPFAULTERRORBOX) in this process.
    unsafe {
        SetErrorMode(GetErrorMode() | LOCAL_ERRORS);
    }
    if reports_disabled() {
        Ok(())
    } else {
        Err("Cannot apply the local error-reporting policy".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_local_fault_reporting_is_disabled_without_registry_changes() {
        struct Restore(u32);
        impl Drop for Restore {
            fn drop(&mut self) {
                // SAFETY: restore the process-local flags captured by this test.
                unsafe {
                    SetErrorMode(self.0);
                }
            }
        }
        // SAFETY: query the current process without pointer arguments.
        let previous = Restore(unsafe { GetErrorMode() });
        configure().unwrap();
        assert!(reports_disabled());
        // SAFETY: query the same process-local flags after configuration.
        assert_eq!(unsafe { GetErrorMode() } & previous.0, previous.0);
    }
}
