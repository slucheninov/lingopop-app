//! The popup as the orchestrator sees it: a view that shows, updates and hides, never focuses.

/// Logical size used until the frontend reports a measured one (mac `PopupLayout.swift`).
pub const INITIAL_POPUP_SIZE: (f64, f64) = (360.0, 100.0);

/// Largest logical side accepted from the webview (a defence against a runaway measurement).
pub const MAX_LOGICAL_SIDE: f64 = 4096.0;

/// Implemented by the app shell (and by `fake::FakePopup`). There is deliberately no focus
/// method: the popup must never take focus from the user's app.
///
/// The orchestrator owns ALL geometry decisions (origin and logical size, see
/// `Orchestrator::popup_resized`); an implementation only converts logical sizes to desktop units
/// with the scale of the monitor under the origin and places the window.
/// Submission under the generation commit only enqueues; native work runs in the consumer.
pub trait PopupView: Send + Sync {
    /// Enqueues only. The consumer must never execute inline, including acknowledgments.
    fn submit(
        &self,
        cmd: crate::PresentationCommand,
    ) -> Result<crate::AckReceiver, crate::PresentationQueueError>;
    /// Called once before the orchestrator is exposed. Sink must precede oneshot resolution.
    fn bind(
        &self,
        fence: std::sync::Arc<crate::PresentationFence>,
        sink: std::sync::Arc<crate::AckSink>,
    );
}
