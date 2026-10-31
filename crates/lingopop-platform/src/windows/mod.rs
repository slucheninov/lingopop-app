//! Windows implementations of the platform traits. Every `unsafe` block of the crate's Windows
//! code lives under this module and carries a `// SAFETY:` comment.

mod capture;
mod clipboard;
mod dpapi;
mod foreground;
mod keys;
mod monitors;
mod ocr;
mod util;

pub use capture::WindowsCapture;
pub use clipboard::WindowsClipboard;
pub use dpapi::WindowsProtector;
pub use foreground::WindowsForeground;
pub use keys::WindowsKeys;
pub use monitors::WindowsMonitors;
pub use ocr::WindowsOcr;

mod popup_window;
pub use popup_window::WindowsPopupWindow;
mod dismiss;
pub use dismiss::WindowsDismissWatcher;
