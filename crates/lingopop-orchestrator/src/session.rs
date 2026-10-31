//! Per-flow state behind one std mutex (never held across an `.await`): the generation counter
//! and abort handle that implement cancellation, the last known cursor / monitors for popup
//! placement, and the current session.

use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_platform::{MonitorInfo, Point, WindowHandle};
use tokio::task::AbortHandle;

/// What the popup actions (Task 4) need about the running operation.
#[derive(Clone)]
pub struct Session {
    pub operation: AIOperation,
    pub text: String,
    /// Text currently displayed in the Result phase, absent on loading / error / close.
    pub last_result: Option<String>,
    pub detected_source: Option<String>,
    pub target_language: String,
    /// The window that was in the foreground before the capture (Replace target).
    pub target_window: Option<WindowHandle>,
    pub primary: ProviderType,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("operation", &self.operation)
            .field("text_chars", &self.text.chars().count())
            .field(
                "result_chars",
                &self.last_result.as_ref().map(|s| s.chars().count()),
            )
            .field("target_window", &self.target_window)
            .field("primary", &self.primary)
            .finish_non_exhaustive()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_debug_never_contains_user_text() {
        let s = Session {
            operation: AIOperation::Translate,
            text: "PRIVATE-INPUT".into(),
            last_result: Some("PRIVATE-RESULT".into()),
            detected_source: None,
            target_language: "German".into(),
            target_window: None,
            primary: ProviderType::Claude,
        };
        let debug = format!("{s:?}");
        assert!(!debug.contains("PRIVATE-"));
    }
}
