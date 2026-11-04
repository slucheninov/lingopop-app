//! `ForegroundWindow` over `NSWorkspace` and a bounded target registry.
//!
//! The handle is NOT a bare pid: it names a generation-stamped registry entry (see
//! `target_registry.rs`) that pins the application's pid, bundle id and launch date and, where
//! Accessibility allows, its focused window. Evicted, expired, exited and pid-reused entries are
//! rejected instead of reaching another application. The logic lives in the portable
//! `macos_targets` module; this file only supplies the AppKit / AX seam.

use std::fmt;
use std::sync::Arc;

use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

use super::ax::{self, AxWindow};
use crate::error::PlatformError;
use crate::foreground::{ForegroundWindow, WindowHandle};
use crate::macos_targets::{AppIdentity, MacApps, TargetTracker};
use crate::sleeper::SystemSleeper;

/// The real `NSWorkspace` / Accessibility implementation of the targeting seam.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemApps;

fn identity(app: &NSRunningApplication) -> Option<AppIdentity> {
    let pid = app.processIdentifier();
    (pid > 0).then(|| AppIdentity {
        pid,
        // Read fresh on every call: this is what makes a reused pid detectable.
        process_start: super::procinfo::start_time_micros(pid),
        bundle_id: app.bundleIdentifier().map(|s| s.to_string()),
        launch: app
            .launchDate()
            .map(|d| (d.timeIntervalSinceReferenceDate() * 1000.0).round() as i64),
    })
}

impl MacApps for SystemApps {
    type Ax = AxWindow;

    fn accessibility_trusted(&self) -> bool {
        ax::process_trusted()
    }

    fn frontmost(&self) -> Option<AppIdentity> {
        autoreleasepool(|_| {
            let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
            identity(&app)
        })
    }

    fn running(&self, pid: i32) -> Option<AppIdentity> {
        autoreleasepool(|_| {
            let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
            if app.isTerminated() {
                return None;
            }
            identity(&app)
        })
    }

    fn focused_window(&self, pid: i32) -> Option<AxWindow> {
        ax::focused_window(pid)
    }

    fn activate(&self, pid: i32, window: Option<&AxWindow>) -> bool {
        autoreleasepool(|_| {
            let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            else {
                return false;
            };
            if let Some(window) = window {
                // Best effort: bring the captured window to the front of its application first.
                ax::raise(window);
            }
            app.activateWithOptions(NSApplicationActivationOptions::empty())
        })
    }
}

pub(crate) type MacTracker = TargetTracker<SystemApps>;

/// macOS foreground control. Clones share one target registry, which is also what
/// `MacLastForeground` must be built from so its handles are valid here.
#[derive(Clone)]
pub struct MacForeground {
    tracker: Arc<MacTracker>,
}

impl fmt::Debug for MacForeground {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MacForeground")
    }
}

impl Default for MacForeground {
    fn default() -> Self {
        Self::new()
    }
}

impl MacForeground {
    pub fn new() -> Self {
        Self {
            tracker: Arc::new(TargetTracker::new(
                SystemApps,
                Arc::new(SystemSleeper::new()),
            )),
        }
    }

    pub(crate) fn tracker(&self) -> Arc<MacTracker> {
        self.tracker.clone()
    }
}

impl ForegroundWindow for MacForeground {
    fn current(&self) -> Option<WindowHandle> {
        self.tracker.current()
    }

    fn restore(&self, window: WindowHandle) -> Result<(), PlatformError> {
        self.tracker.restore(window)
    }

    fn is_elevated(&self, _window: WindowHandle) -> bool {
        false
    }

    fn is_current_target(&self, window: WindowHandle) -> Result<bool, PlatformError> {
        self.tracker.is_current_target(window)
    }
}
