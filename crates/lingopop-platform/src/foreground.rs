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
    /// TODO(manual checklist): verify Capture / Replace against an elevated Notepad ("Run as
    /// administrator") from a non-elevated LingoPop: it must report `InputBlocked`.
    fn is_elevated(&self, window: WindowHandle) -> bool;
}

/// UIPI drops input sent to a window whose process runs elevated when ours does not.
#[doc(hidden)]
pub fn integrity_blocks(target_elevated: bool, self_elevated: bool) -> bool {
    target_elevated && !self_elevated
}

/// Win32 `ERROR_ACCESS_DENIED`.
#[doc(hidden)]
pub const ERROR_ACCESS_DENIED: i32 = 5;

/// The Windows `is_elevated` decision from the probe results (Win32 error codes on failure):
/// `open_process` = `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`, `open_token` =
/// `OpenProcessToken(TOKEN_QUERY)` then `Ok(None)` when `GetTokenInformation(TokenElevation)`
/// fails (ignored when `open_process` failed). From a non-elevated process, opening an elevated
/// process usually succeeds but opening its token fails with access denied, so access denied on
/// either step while we are not elevated means "runs above us"; any other failure is not
/// elevated (never block on a guess).
#[doc(hidden)]
pub fn elevation_blocks(
    open_process: Result<(), i32>,
    open_token: Result<Option<bool>, i32>,
    self_elevated: bool,
) -> bool {
    let denied = |code: i32| code == ERROR_ACCESS_DENIED && !self_elevated;
    match (open_process, open_token) {
        (Err(code), _) => denied(code),
        (Ok(()), Err(code)) => denied(code),
        (Ok(()), Ok(Some(target))) => integrity_blocks(target, self_elevated),
        (Ok(()), Ok(None)) => false,
    }
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

    const DENIED: i32 = ERROR_ACCESS_DENIED;

    #[test]
    fn elevation_blocks_access_denied_on_open_process_when_we_are_not_elevated() {
        assert!(elevation_blocks(Err(DENIED), Err(DENIED), false));
        assert!(!elevation_blocks(Err(DENIED), Err(DENIED), true));
        assert!(!elevation_blocks(Err(87), Ok(Some(true)), false));
    }

    #[test]
    fn elevation_blocks_access_denied_on_open_token_when_we_are_not_elevated() {
        assert!(elevation_blocks(Ok(()), Err(DENIED), false));
        assert!(!elevation_blocks(Ok(()), Err(DENIED), true));
        assert!(!elevation_blocks(Ok(()), Err(6), false));
    }

    #[test]
    fn elevation_blocks_compares_a_readable_token() {
        assert!(elevation_blocks(Ok(()), Ok(Some(true)), false));
        assert!(!elevation_blocks(Ok(()), Ok(Some(true)), true));
        assert!(!elevation_blocks(Ok(()), Ok(Some(false)), false));
        // GetTokenInformation failed: never block on a guess.
        assert!(!elevation_blocks(Ok(()), Ok(None), false));
    }
}
