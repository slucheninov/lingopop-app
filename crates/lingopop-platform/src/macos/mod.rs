//! macOS DEV implementations of the platform traits, so the owner can run the app on the dev Mac.
//! Never shipped. Every `unsafe` block of the crate's macOS code lives under this module and
//! carries a `// SAFETY:` comment.

mod capture;
mod clipboard;
mod foreground;
mod keys;
mod monitors;
mod ocr;

pub use capture::MacCapture;
pub use clipboard::MacClipboard;
pub use foreground::MacForeground;
pub use keys::MacKeys;
pub use monitors::MacMonitors;
pub use ocr::MacOcr;
