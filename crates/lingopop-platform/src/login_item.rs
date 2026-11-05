//! Launch at login. Windows stores a quoted executable path under HKCU Run; macOS registers the
//! main app through `SMAppService.mainApp` (see `macos/login_service.rs`) and ignores `exe`.
use crate::PlatformError;
use std::path::Path;

/// The `PlatformError::Unsupported` payload of every "launch at login is not available here"
/// outcome (the macOS app is not an installed bundle, or the service cannot be found). The
/// message mapping keys off it.
pub const UNSUPPORTED_LOGIN_ITEM: &str = "launch at login";

/// `PlatformError::Unsupported` payload when the system cannot find the login item service of an
/// app that IS in an installed bundle (`SMAppServiceStatus.notFound`): not a location problem, so
/// the message must not tell the user to move the app.
pub const UNSUPPORTED_LOGIN_ITEM_NOT_FOUND: &str = "launch at login service not found";

/// What the login item looks like right now, read without changing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemStatus {
    Enabled,
    /// Registered, but the user still has to allow it (macOS: System Settings > Login Items).
    RequiresApproval,
    NotRegistered,
    /// No state to report: Windows (the Run value needs no approval) and a macOS service the
    /// system cannot find.
    Unknown,
}

pub trait LoginItem: Send + Sync {
    fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError>;

    /// The current state, READ ONLY: never registers, unregisters or writes anything, so the
    /// shell can poll it (e.g. when Settings regains focus after the user approved the item).
    /// Blocking like `set_enabled`: call it off the main thread.
    fn status(&self) -> LoginItemStatus {
        LoginItemStatus::Unknown
    }

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
