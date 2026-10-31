//! The popup as the orchestrator sees it: a view that shows, updates and hides, never focuses.

use lingopop_platform::{Point, Rect};

use crate::view_model::PopupViewModel;

/// Logical size used to place a freshly shown popup (mac `PopupLayout.swift`); the shell
/// measures and resizes the real window itself afterwards.
pub const INITIAL_POPUP_SIZE: (f64, f64) = (360.0, 100.0);

/// Implemented by the app shell (and by `fake::FakePopup`). There is deliberately no focus
/// method: the popup must never take focus from the user's app.
/// Calls run under the generation commit lock: implementations must not synchronously call
/// back into `Orchestrator`. The Tauri shell emits events / IPC asynchronously.
pub trait PopupView: Send + Sync {
    /// Shows the popup at `origin` (physical px, top-left). Called when not visible, and for the
    /// first Loading of a Screen Translate flow even when visible (the new region may be on
    /// another monitor); other flows only `update` a visible popup, like mac `show`.
    fn show_at(&self, origin: Point, model: &PopupViewModel);
    /// Replaces the content of the visible popup.
    fn update(&self, model: &PopupViewModel);
    /// The measured content size in LOGICAL pixels. Stores it; when the popup is visible,
    /// re-places it at its current origin and returns the new physical rect, else returns
    /// `None` and shows nothing (a resize must never resurrect a hidden popup). The
    /// orchestrator calls this under the commit lock, only while the popup is visible.
    fn resize(&self, size_logical: (f64, f64)) -> Option<Rect>;
    fn hide(&self);
    fn is_visible(&self) -> bool;
}
