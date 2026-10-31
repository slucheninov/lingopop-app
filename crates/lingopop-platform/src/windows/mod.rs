//! Windows implementations of the platform traits. Every `unsafe` block of the crate's Windows
//! code lives under this module and carries a `// SAFETY:` comment.

mod clipboard;
mod foreground;
mod keys;
mod util;

pub use clipboard::WindowsClipboard;
pub use foreground::WindowsForeground;
pub use keys::WindowsKeys;
