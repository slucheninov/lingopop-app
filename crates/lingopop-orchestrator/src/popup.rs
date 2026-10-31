//! The popup as the orchestrator sees it: a view that shows, updates and hides, never focuses.

use lingopop_platform::Point;

use crate::view_model::PopupViewModel;

/// Logical size used until the frontend reports a measured one (mac `PopupLayout.swift`).
pub const INITIAL_POPUP_SIZE: (f64, f64) = (360.0, 100.0);

/// Largest logical side accepted from the webview (a defence against a runaway measurement).
pub const MAX_LOGICAL_SIDE: f64 = 4096.0;

/// Implemented by the app shell (and by `fake::FakePopup`). There is deliberately no focus
/// method: the popup must never take focus from the user's app.
///
/// The orchestrator owns ALL geometry decisions (origin and logical size, see
/// `Orchestrator::popup_resized`); an implementation only converts logical to physical pixels
/// with the scale of the monitor under the origin and places the window.
/// Calls run under the generation commit lock: implementations must not synchronously call
/// back into `Orchestrator`. The Tauri shell emits events / IPC asynchronously.
pub trait PopupView: Send + Sync {
    /// Shows the popup at `origin` (physical px, top-left) with `size_logical`. Called when not
    /// visible, and for the first Loading of a Screen Translate flow even when visible (the new
    /// region may be on another monitor); other flows only `update` a visible popup, like mac
    /// `show`.
    fn show_at(&self, origin: Point, size_logical: (f64, f64), model: &PopupViewModel);
    /// Replaces the content of the visible popup.
    fn update(&self, model: &PopupViewModel);
    /// Moves and resizes the visible popup to `origin` / `size_logical`. Called by the
    /// orchestrator under the commit lock, only while the popup is visible (a resize must never
    /// resurrect a hidden popup).
    fn resize(&self, origin: Point, size_logical: (f64, f64));
    fn hide(&self);
    fn is_visible(&self) -> bool;
}
