//! Operating-system privacy permissions the app needs on macOS: Accessibility (to read the
//! selection and paste) and Screen Recording (Screen Translate). Windows has neither, so its
//! default is [`AlwaysGranted`].
//!
//! The seam keeps three things apart: `state` never prompts (bootstrap, settings reads and tray
//! rebuilds use it), `request` is a user-action that may show the system prompt, and
//! `open_settings` jumps to the matching privacy pane. All three may block briefly, so callers run
//! them in the platform gate or `spawn_blocking`, never on the main thread.

use crate::error::PlatformError;

/// What can be granted. Fixed set: the webview only ever names one of these, never a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PermissionKind {
    Accessibility,
    ScreenRecording,
}

impl PermissionKind {
    pub const ALL: [PermissionKind; 2] = [
        PermissionKind::Accessibility,
        PermissionKind::ScreenRecording,
    ];

    /// The name macOS shows in System Settings (also the English catalog key).
    pub fn display_name(self) -> &'static str {
        match self {
            PermissionKind::Accessibility => "Accessibility",
            PermissionKind::ScreenRecording => "Screen Recording",
        }
    }

    /// The stable id of the IPC wire format.
    pub fn id(self) -> &'static str {
        match self {
            PermissionKind::Accessibility => "accessibility",
            PermissionKind::ScreenRecording => "screenRecording",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

/// `Restricted` (a profile or parental control forbids it) is reported only when an API says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionState {
    NotDetermined,
    Denied,
    Granted,
    Restricted,
}

impl PermissionState {
    /// The stable id of the IPC wire format.
    pub fn id(self) -> &'static str {
        match self {
            PermissionState::NotDetermined => "notDetermined",
            PermissionState::Denied => "denied",
            PermissionState::Granted => "granted",
            PermissionState::Restricted => "restricted",
        }
    }
}

pub trait Permissions: Send + Sync {
    /// The current state. Never prompts and never opens a window.
    fn state(&self, kind: PermissionKind) -> PermissionState;
    /// A user-triggered request: may show the system prompt (once). Returns the state right
    /// after the call, which is usually not yet `Granted`: the user answers asynchronously.
    fn request(&self, kind: PermissionKind) -> Result<PermissionState, PlatformError>;
    /// Opens the privacy pane of `kind` in System Settings (a fixed URL chosen by the platform).
    fn open_settings(&self, kind: PermissionKind) -> Result<(), PlatformError>;
}

/// The Windows default and the default of tests: everything is granted, nothing prompts.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysGranted;

impl Permissions for AlwaysGranted {
    fn state(&self, _kind: PermissionKind) -> PermissionState {
        PermissionState::Granted
    }
    fn request(&self, _kind: PermissionKind) -> Result<PermissionState, PlatformError> {
        Ok(PermissionState::Granted)
    }
    fn open_settings(&self, _kind: PermissionKind) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// The gate for a user-triggered operation: `Granted` passes; `NotDetermined` asks once (the
/// only case that may prompt) and passes only when the answer is already `Granted`; `Denied` and
/// `Restricted` never prompt and fail with `PermissionDenied(kind)`. Blocking: call it from a
/// gate job. `log` receives fixed lines (`permission requested: <id>`, `permission missing: <id>`),
/// never user data.
pub fn ensure_permission(
    permissions: &dyn Permissions,
    kind: PermissionKind,
    log: &dyn Fn(&str),
) -> Result<(), PlatformError> {
    let granted = match permissions.state(kind) {
        PermissionState::Granted => true,
        PermissionState::NotDetermined => {
            log(&format!("permission requested: {}", kind.id()));
            permissions.request(kind)? == PermissionState::Granted
        }
        PermissionState::Denied | PermissionState::Restricted => false,
    };
    if granted {
        Ok(())
    } else {
        log(&format!("permission missing: {}", kind.id()));
        Err(PlatformError::PermissionDenied(kind))
    }
}
