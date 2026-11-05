//! Launch at login. Windows stores a quoted executable path under HKCU Run; macOS registers the
//! main app through `SMAppService.mainApp` (see `macos/login_service.rs`) and ignores `exe`.
use crate::PlatformError;
use std::path::Path;

/// The `PlatformError::Unsupported` payload of every "launch at login is not available here"
/// outcome (the macOS app is not an installed bundle, or the service cannot be found). The
/// message mapping keys off it.
pub const UNSUPPORTED_LOGIN_ITEM: &str = "launch at login";

pub trait LoginItem: Send + Sync {
    fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError>;

    /// Opens the system pane where the user approves the login item (macOS: System Settings,
    /// Login Items). The target is fixed by the platform. Windows has no such pane.
    fn open_settings(&self) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported(UNSUPPORTED_LOGIN_ITEM))
    }
}

/// Registry Run value: the executable path wrapped in double quotes.
pub fn run_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}
