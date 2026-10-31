//! OS integration for LingoPop: traits, in-memory fakes and the Windows / macOS implementations.
//!
//! `unsafe` is confined to `src/windows/` and `src/macos/` (added by later tasks with a local
//! `#[allow(unsafe_code)]`); everything else is denied at the crate level.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod clipboard;
pub mod error;
pub mod fake;
pub mod foreground;
pub mod inject;
pub mod keys;
pub mod selection;
pub mod sleeper;

pub use clipboard::{Clipboard, ClipboardEntry, ClipboardFormat, ClipboardSnapshot};
pub use error::{LogFn, PlatformError};
pub use foreground::{ForegroundWindow, WindowHandle};
pub use inject::{replace_selection, ReplaceOptions, ReplaceOutcome};
pub use keys::KeySynth;
pub use selection::{capture_selection, CaptureOptions};
pub use sleeper::{retry_busy, Sleeper, SystemSleeper};
