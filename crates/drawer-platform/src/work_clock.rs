//! System working-state clock: sleep/hibernation is excluded by Windows itself.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn milliseconds() -> Option<u64> {
    let mut ticks = 0u64;
    // SAFETY: the OS writes one u64 through a valid, uniquely borrowed stack pointer.
    unsafe { windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime(&mut ticks) }
        .as_bool()
        .then_some(ticks / 10_000)
}
#[cfg(not(windows))]
pub fn milliseconds() -> Option<u64> {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    Some(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64,
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_work_clock_is_available_and_monotonic() {
        let before = super::milliseconds().expect("System working-state clock unavailable");
        std::thread::sleep(std::time::Duration::from_millis(35));
        let after = super::milliseconds().unwrap();
        assert!(after >= before);
        assert!(after - before >= 10);
    }
}
