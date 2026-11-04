use crate::{PlatformError, SystemInfo};
use objc2_foundation::NSLocale;
use std::{
    process::Command,
    time::{Duration, Instant},
};
pub struct MacSystem;
impl SystemInfo for MacSystem {
    fn ui_languages(&self) -> Vec<String> {
        NSLocale::preferredLanguages()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        match Command::new("open").arg(url).status() {
            Ok(s) if s.success() => Ok(()),
            _ => Err(PlatformError::Os {
                api: "open",
                code: 1,
            }),
        }
    }
    fn wait_for_process_exit(&self, pid: u32, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| !s.success())
            {
                return true;
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return false;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

/// Called only by the shell's posted main-thread job.
pub fn activate_app() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}
