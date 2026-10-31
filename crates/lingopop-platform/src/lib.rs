//! OS integration for LingoPop: traits, in-memory fakes and the Windows / macOS implementations.
//!
//! `unsafe` is confined to `src/windows/` and `src/macos/` (each with a local
//! `#[allow(unsafe_code)]`); everything else is denied at the crate level.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod capture;
pub mod clipboard;
#[cfg(not(windows))]
pub mod dev_protector;
pub mod error;
pub mod fake;
pub mod foreground;
pub mod hotkey;
pub mod inject;
pub mod keys;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos;
pub mod monitors;
pub mod native;
pub mod ocr;
pub mod selection;
pub mod sleeper;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(target_os = "macos")]
pub use crate::macos::{MacCapture, MacClipboard, MacForeground, MacKeys, MacMonitors, MacOcr};
#[cfg(windows)]
pub use crate::windows::{
    WindowsCapture, WindowsClipboard, WindowsForeground, WindowsKeys, WindowsMonitors, WindowsOcr,
    WindowsProtector,
};
pub use capture::{CapturedImage, ScreenCapture};
pub use clipboard::{Clipboard, ClipboardEntry, ClipboardFormat, ClipboardSnapshot};
#[cfg(not(windows))]
pub use dev_protector::DevPlaintextProtector;
pub use error::{LogFn, PlatformError};
pub use foreground::{ForegroundWindow, WindowHandle};
pub use hotkey::{to_chord, HotkeyEvent, HotkeyId, HotkeyService};
#[cfg(any(windows, target_os = "macos"))]
pub use hotkey::{to_global_hotkey, GlobalHotkeyService};
pub use inject::{replace_selection, ReplaceOptions, ReplaceOutcome};
pub use keys::KeySynth;
pub use monitors::{
    clamp_to_work_area, monitor_at, physical_size, validate_region, virtual_screen, MonitorInfo,
    Monitors, Point, Rect, Size,
};
#[cfg(any(windows, target_os = "macos"))]
pub use native::native;
pub use native::Platform;
pub use ocr::{select_best, Ocr, OcrCandidate, OcrText};
pub use selection::{capture_selection, CaptureOptions};
pub use sleeper::{retry_busy, Sleeper, SystemSleeper};
