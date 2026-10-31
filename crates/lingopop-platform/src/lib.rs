//! OS integration for LingoPop: traits, in-memory fakes and the Windows / macOS implementations.
//!
//! `unsafe` is confined to `src/windows/` and `src/macos/` (added by later tasks with a local
//! `#[allow(unsafe_code)]`); everything else is denied at the crate level.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod error;
pub mod fake;
pub mod sleeper;

pub use error::{LogFn, PlatformError};
pub use sleeper::{Sleeper, SystemSleeper};
