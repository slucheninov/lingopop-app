//! In-memory fakes of the platform traits (always compiled, used by tests here and in later phases).

mod capture;
mod clipboard;
mod foreground;
mod hotkey;
mod keys;
mod log;
mod monitors;
mod ocr;
mod sleeper;

pub use capture::FakeCapture;
pub use clipboard::{ClipboardCall, FakeClipboard, FakeOp, Mutation};
pub use foreground::FakeForeground;
pub use hotkey::FakeHotkeyService;
pub use keys::{CopyBehavior, FakeKeys, KeyCall};
pub use log::{Event, EventLog, LoggedEvent};
pub use monitors::FakeMonitors;
pub use ocr::FakeOcr;
pub use sleeper::FakeSleeper;
