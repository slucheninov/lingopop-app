//! Native popup visibility without activating the user's foreground window.
use crate::{PlatformError, Rect, WindowHandle};
pub trait NoActivateWindow: Send + Sync {
    fn show_at(&self, window: WindowHandle, rect: Rect) -> Result<(), PlatformError>;
    fn hide(&self, window: WindowHandle) -> Result<(), PlatformError>;
}
