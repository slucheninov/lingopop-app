//! In-memory fakes of the platform traits (always compiled, used by tests here and in later phases).

mod capture;
mod clipboard;
mod foreground;
mod hotkey;
mod keys;
mod last_foreground;
mod log;
mod monitors;
mod ocr;
mod pasteboard;
mod permissions;
mod sleeper;

pub use capture::FakeCapture;
pub use clipboard::{ClipboardCall, FakeClipboard, FakeOp, Mutation};
pub use foreground::FakeForeground;
pub use hotkey::FakeHotkeyService;
pub use keys::{CopyBehavior, FakeKeys, KeyCall};
pub use last_foreground::FakeLastForeground;
pub use log::{Event, EventLog, LoggedEvent};
pub use monitors::FakeMonitors;
pub use ocr::FakeOcr;
pub use pasteboard::FakePasteboard;
pub use permissions::FakePermissions;
pub use sleeper::FakeSleeper;

mod popup_window;
pub use popup_window::FakePopupWindow;
mod dismiss;
pub use dismiss::DismissCall;
pub use dismiss::FakeDismissWatcher;
pub use popup_window::PopupWindowCall;

mod system;
pub use system::FakeSystem;

mod login_item;
pub use login_item::FakeLoginItem;

mod overlay;
pub use overlay::FakeOverlay;

mod main_thread;
pub use main_thread::{on_fake_main, FakeMainThread};
