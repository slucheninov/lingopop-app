//! macOS adapters, including Keychain secrets and the remaining dev stubs.
//! Every `unsafe` block of the crate's macOS code lives under this module and
//! carries a `// SAFETY:` comment.

mod capture;
mod clipboard;
mod foreground;
mod keys;
mod last_foreground;
mod monitors;
mod ocr;

pub use capture::MacCapture;
pub use clipboard::MacClipboard;
pub use foreground::MacForeground;
pub use keys::MacKeys;
pub use last_foreground::MacLastForeground;
pub use monitors::MacMonitors;
pub use ocr::MacOcr;

mod popup_window;
pub use popup_window::MacDismissWatcher;
pub use popup_window::MacPopupWindow;

mod system;
pub use system::MacSystem;

mod login_item;
pub use login_item::MacLoginItem;

mod overlay;
pub use overlay::MacOverlay;

pub mod keychain;
mod secrets;
pub use secrets::MacProtector;
