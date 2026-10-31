//! In-memory fakes of the platform traits (always compiled, used by tests here and in later phases).

mod clipboard;
mod foreground;
mod keys;
mod log;
mod sleeper;

pub use clipboard::{ClipboardCall, FakeClipboard, FakeOp, Mutation};
pub use foreground::FakeForeground;
pub use keys::{CopyBehavior, FakeKeys, KeyCall};
pub use log::{Event, EventLog, LoggedEvent};
pub use sleeper::FakeSleeper;
