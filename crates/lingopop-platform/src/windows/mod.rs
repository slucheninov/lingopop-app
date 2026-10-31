//! Windows implementations of the platform traits. Every `unsafe` block of the crate's Windows
//! code lives under this module and carries a `// SAFETY:` comment.

mod capture;
mod clipboard;
mod foreground;
mod keys;
mod monitors;
mod util;

pub use capture::WindowsCapture;
pub use clipboard::WindowsClipboard;
pub use foreground::WindowsForeground;
pub use keys::WindowsKeys;
pub use monitors::WindowsMonitors;
