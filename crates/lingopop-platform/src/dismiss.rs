//! Dismissal for a popup that never receives keyboard focus.
use crate::{PlatformError, Point, Rect};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissEvent {
    Escape,
    ClickOutside,
}
/// All methods must be non-blocking: the orchestrator invokes them while holding its
/// session mutex. Native implementations post commands / publish geometry without waiting
/// for hooks to install or uninstall. Receivers cloned by events() share one event stream.
pub trait DismissWatcher: Send + Sync {
    fn start(&self, popup_rect: Rect) -> Result<(), PlatformError>;
    fn update_rect(&self, popup_rect: Rect);
    fn stop(&self);
    fn events(&self) -> crossbeam_channel::Receiver<DismissEvent>;
}
pub fn classify_click(point: Point, popup: Rect) -> Option<DismissEvent> {
    (!popup.contains(point)).then_some(DismissEvent::ClickOutside)
}
