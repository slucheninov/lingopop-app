//! Foreground-window seam.

use crate::error::PlatformError;

/// Opaque window identifier (an `HWND` or a pid, depending on the platform).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(pub u64);

pub trait ForegroundWindow: Send + Sync {
    fn current(&self) -> Option<WindowHandle>;
    fn restore(&self, window: WindowHandle) -> Result<(), PlatformError>;
    /// The window's process runs at a higher integrity level than ours, so UIPI would silently
    /// drop our synthesized input (`SendInput` still reports success).
    fn is_elevated(&self, window: WindowHandle) -> bool;
}
