//! `ForegroundWindow` over `NSWorkspace`: the handle is the frontmost application's pid.

use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

use crate::error::PlatformError;
use crate::foreground::{ForegroundWindow, WindowHandle};

/// Dev only: tracks applications, not windows, and never reports elevation.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacForeground;

impl MacForeground {
    pub fn new() -> Self {
        Self
    }
}

impl ForegroundWindow for MacForeground {
    fn current(&self) -> Option<WindowHandle> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        u64::try_from(app.processIdentifier())
            .ok()
            .filter(|&pid| pid > 0)
            .map(WindowHandle)
    }

    fn restore(&self, window: WindowHandle) -> Result<(), PlatformError> {
        let not_running = PlatformError::Os {
            api: "NSRunningApplication.activate",
            code: 0,
        };
        let pid = i32::try_from(window.0).map_err(|_| not_running.clone())?;
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .ok_or_else(|| not_running.clone())?;
        if app.activateWithOptions(NSApplicationActivationOptions::empty()) {
            Ok(())
        } else {
            Err(not_running)
        }
    }

    fn is_elevated(&self, _window: WindowHandle) -> bool {
        false
    }
}
