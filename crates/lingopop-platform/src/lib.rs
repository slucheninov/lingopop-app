//! OS integration for LingoPop: traits, in-memory fakes and the Windows / macOS implementations.
//!
//! `unsafe` is confined to `src/windows/` and `src/macos/` (each with a local
//! `#[allow(unsafe_code)]`); everything else is denied at the crate level.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod capture;
pub mod clipboard;
#[cfg(any(test, feature = "dev-plaintext"))]
pub mod dev_protector;
pub mod dismiss;
pub mod error;
pub mod fake;
pub mod foreground;
pub mod hotkey;
pub mod inject;
pub mod keys;
pub mod last_foreground;
pub mod login_item;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos;
#[path = "macos/dismiss.rs"]
pub mod macos_dismiss;
#[path = "macos/geometry.rs"]
pub mod macos_geometry;
#[path = "macos/login_service.rs"]
pub mod macos_login;
#[path = "macos/overlay.rs"]
pub mod macos_overlay;
#[path = "macos/presentation.rs"]
pub mod macos_presentation;
#[path = "macos/screen.rs"]
pub mod macos_screen;
#[path = "macos/target_registry.rs"]
pub mod macos_targets;
pub mod main_thread;
pub mod monitors;
pub mod native;
pub mod ocr;
pub mod pasteboard;
pub mod permissions;
pub mod popup_window;
pub mod secrets_aead;
pub mod selection;
pub mod sleeper;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(target_os = "macos")]
pub use crate::macos::{
    MacCapture, MacClipboard, MacDismissWatcher, MacForeground, MacKeys, MacLastForeground,
    MacLoginItem, MacMonitors, MacOcr, MacOverlay, MacPermissions, MacPopupWindow, MacProtector,
    NsPasteboard, SmMainApp,
};
#[cfg(windows)]
pub use crate::windows::{
    WindowsCapture, WindowsClipboard, WindowsDismissWatcher, WindowsForeground, WindowsKeys,
    WindowsLastForeground, WindowsLoginItem, WindowsMonitors, WindowsOcr, WindowsOverlay,
    WindowsPopupWindow, WindowsProtector,
};
pub use capture::{CapturedImage, ScreenCapture};
pub use clipboard::{Clipboard, ClipboardEntry, ClipboardFormat, ClipboardSnapshot};
#[cfg(any(test, feature = "dev-plaintext"))]
pub use dev_protector::DevPlaintextProtector;
pub use error::{set_platform_log, LogFn, PlatformError};
pub use foreground::{ForegroundWindow, WindowHandle};
pub use hotkey::{to_chord, HotkeyEvent, HotkeyId, HotkeyService};
#[cfg(any(windows, target_os = "macos"))]
pub use hotkey::{to_global_hotkey, GlobalHotkeyService};
pub use inject::{bring_to_front, replace_selection, ReplaceOptions, ReplaceOutcome};
pub use keys::KeySynth;
pub use last_foreground::{LastForeground, NoLastForeground};
pub use login_item::{run_value, LoginItem};
pub use monitors::{
    clamp_to_work_area, monitor_at, physical_size, validate_region, virtual_screen, MonitorInfo,
    Monitors, Point, Rect, Size,
};
pub use native::Platform;
#[cfg(any(windows, target_os = "macos"))]
pub use native::{native, native_system};
pub use ocr::{select_best, text_in_reading_order, Ocr, OcrCandidate, OcrText};
pub use permissions::{
    ensure_permission, AlwaysGranted, PermissionKind, PermissionState, Permissions,
};
pub use selection::{capture_selection, CaptureOptions};
pub use sleeper::{retry_busy, Sleeper, SystemSleeper};

pub use dismiss::{classify_click, DismissEvent, DismissWatcher};
pub use main_thread::MainThread;
pub use popup_window::NoActivateWindow;

pub mod system;
pub use system::SystemInfo;

pub mod overlay;
pub use overlay::RegionOverlay;

#[cfg(target_os = "macos")]
pub use macos::keychain::LoginKeyStore;

#[cfg(target_os = "macos")]
pub use macos::activate_app;
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use macos::OwnerRunWindow;
#[cfg(target_os = "macos")]
pub use macos::{configure_popup_panel, AppKitMonitors, AppKitOverlay};
