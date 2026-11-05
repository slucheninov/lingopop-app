//! Dismissal for a popup that never receives keyboard focus.
use crate::{PlatformError, Point, Rect};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissEvent {
    Escape,
    ClickOutside,
}
/// All methods except `shutdown` must be non-blocking: the orchestrator invokes them while
/// holding its session mutex. Native implementations post commands / publish geometry without waiting
/// for hooks to install or uninstall. Receivers cloned by events() share one event stream.
pub trait DismissWatcher: Send + Sync {
    fn start(&self, popup_rect: Rect) -> Result<(), PlatformError>;
    fn update_rect(&self, popup_rect: Rect);
    fn stop(&self);
    fn events(&self) -> crossbeam_channel::Receiver<DismissEvent>;
    /// Hands the watcher the main-thread dispatcher it installs native monitors through
    /// (macOS: `NSEvent` monitors are main-thread only). Called once by the shell at setup;
    /// the default ignores it (Windows hooks run on their own thread).
    fn bind_main_thread(&self, _main: std::sync::Arc<dyn crate::MainThread>) {}
    /// False when Escape cannot be observed (macOS without Accessibility trust); click-outside
    /// dismissal still works then. Never prompts.
    fn escape_available(&self) -> bool {
        true
    }
    /// Process teardown: stops and removes every native monitor before returning; later
    /// `start` calls fail. May wait (bounded) for main-thread cleanup, so call it holding no
    /// lock and never from the orchestrator; inline when already on the main thread.
    fn shutdown(&self) {
        self.stop();
    }
}
pub fn classify_click(point: Point, popup: Rect) -> Option<DismissEvent> {
    (!popup.contains(point)).then_some(DismissEvent::ClickOutside)
}
