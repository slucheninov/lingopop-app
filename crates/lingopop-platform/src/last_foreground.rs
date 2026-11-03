//! Last external foreground window: the window the user was in before they touched our tray.

use crate::foreground::WindowHandle;

/// Remembers the most recent foreground window that is not ours and not part of the shell.
/// The tray menu and our own windows take the foreground when the user clicks them, so the
/// foreground at click time is useless for a tray-triggered operation; this is the window to
/// bring back first.
pub trait LastForeground: Send + Sync {
    /// The remembered window, or `None` when nothing is remembered or it no longer exists.
    fn last_external(&self) -> Option<WindowHandle>;
}

/// Window classes of the shell that become the foreground when the user clicks the tray or an
/// overflow menu; they must never replace the remembered window.
const SHELL_CLASSES: [&str; 8] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "XamlExplorerHostIslandWindow",
    "Progman",
    "WorkerW",
    "#32768",
];

/// Whether a window that just became the foreground may be remembered: not one of our own
/// windows (`pid == own_pid`) and not a shell class.
#[doc(hidden)]
pub fn is_trackable(class_name: &str, pid: u32, own_pid: u32) -> bool {
    pid != own_pid && !SHELL_CLASSES.contains(&class_name)
}

/// Remembers nothing (used when the Windows hook could not be installed).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLastForeground;

impl LastForeground for NoLastForeground {
    fn last_external(&self) -> Option<WindowHandle> {
        None
    }
}
