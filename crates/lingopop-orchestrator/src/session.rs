//! Per-flow state behind one std mutex (never held across an `.await`): the generation counter
//! and abort handle that implement cancellation, the last known cursor / monitors for popup
//! placement, and the current session.

use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_platform::{MonitorInfo, Point, WindowHandle};
use tokio::task::AbortHandle;

/// What the popup actions (Task 4) need about the running operation.
#[derive(Debug, Clone)]
pub struct Session {
    pub operation: AIOperation,
    pub text: String,
    pub detected_source: Option<String>,
    pub target_language: String,
    /// The window that was in the foreground before the capture (Replace target).
    pub target_window: Option<WindowHandle>,
    pub primary: ProviderType,
}

#[derive(Debug)]
pub(crate) struct SessionState {
    pub generation: u64,
    pub current: Option<AbortHandle>,
    pub session: Option<Session>,
    pub cursor: Point,
    pub monitors: Vec<MonitorInfo>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            generation: 0,
            current: None,
            session: None,
            cursor: Point { x: 0, y: 0 },
            monitors: Vec::new(),
        }
    }
}
