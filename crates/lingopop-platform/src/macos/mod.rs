//! macOS adapters, including Keychain secrets and the remaining dev stubs.
//! Every `unsafe` block of the crate's macOS code lives under this module and
//! carries a `// SAFETY:` comment.

mod ax;
mod capture;
mod clipboard;
mod foreground;
mod keys;
mod last_foreground;
mod monitors;
mod ocr;
mod procinfo;

pub use capture::MacCapture;
pub use clipboard::{MacClipboard, NsPasteboard};
pub use foreground::MacForeground;
pub use keys::MacKeys;
pub use last_foreground::MacLastForeground;
pub use monitors::MacMonitors;
pub use ocr::MacOcr;

mod event_monitors;
pub use event_monitors::AppKitMonitors;
mod popup_window;
pub use popup_window::{configure_popup_panel, MacPopupWindow};
/// The production macOS dismiss watcher: passive `NSEvent` monitors.
pub type MacDismissWatcher = crate::macos_dismiss::MacDismissWatcher<AppKitMonitors>;

/// True on the process main thread.
pub fn on_main_thread() -> bool {
    objc2::MainThreadMarker::new().is_some()
}

mod system;
pub use system::MacSystem;

mod login_item;
pub use login_item::MacLoginItem;

mod overlay;
pub use overlay::MacOverlay;

mod permissions;
pub use permissions::MacPermissions;

pub mod keychain;
mod secrets;
pub use secrets::MacProtector;

pub use system::activate_app;
