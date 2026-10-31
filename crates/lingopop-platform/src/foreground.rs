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

/// UIPI drops input sent to a window whose process runs elevated when ours does not.
#[doc(hidden)]
pub fn integrity_blocks(target_elevated: bool, self_elevated: bool) -> bool {
    target_elevated && !self_elevated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_blocks_only_when_target_is_elevated_and_we_are_not() {
        assert!(integrity_blocks(true, false));
        assert!(!integrity_blocks(true, true));
        assert!(!integrity_blocks(false, false));
        assert!(!integrity_blocks(false, true));
    }
}
