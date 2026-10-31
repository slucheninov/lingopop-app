//! The popup as the orchestrator sees it: a view that shows, updates and hides, never focuses.

use lingopop_platform::Point;

use crate::view_model::PopupViewModel;

/// Logical size used to place a freshly shown popup (mac `PopupLayout.swift`); the shell
/// measures and resizes the real window itself afterwards.
pub const INITIAL_POPUP_SIZE: (f64, f64) = (360.0, 100.0);

/// Implemented by the app shell (and by `fake::FakePopup`). There is deliberately no focus
/// method: the popup must never take focus from the user's app.
pub trait PopupView: Send + Sync {
    /// Shows the popup at `origin` (physical px, top-left). Called only when not visible, like
    /// mac `show` not repositioning a visible popup.
    fn show_at(&self, origin: Point, model: &PopupViewModel);
    /// Replaces the content of the visible popup.
    fn update(&self, model: &PopupViewModel);
    fn hide(&self);
    fn is_visible(&self) -> bool;
}
