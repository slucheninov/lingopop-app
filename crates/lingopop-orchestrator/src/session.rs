//! Per-flow state behind one std mutex (never held across an `.await`): the generation counter
//! and abort handle that implement cancellation, the last known cursor / monitors for popup
//! placement, and the current session.

use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use std::sync::Arc;

use lingopop_platform::{CapturedImage, MonitorInfo, Point, WindowHandle};
use tokio::task::AbortHandle;

use crate::popup::INITIAL_POPUP_SIZE;
use crate::view_model::PopupViewModel;

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
    /// The captured Screen Translate region, kept for the vision retry. Dropped on close and
    /// whenever a non-ScreenTranslate operation starts.
    pub image: Option<Arc<CapturedImage>>,
    /// The Result / OCR view model on screen, so a failed Copy or Replace can show the same
    /// result again with a notice. Cleared when any other phase is presented.
    pub last_model: Option<PopupViewModel>,
    /// Set after a Replace whose outcome is unknown (the gate timed out or the job panicked,
    /// so the paste may still happen). Replace is then a no-op and hidden for the rest of the
    /// session, so the text cannot be inserted twice. Only a new session resets it.
    pub replace_disabled: bool,
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
            .field("image", &self.image)
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
    /// The popup's LOGICAL size: `INITIAL_POPUP_SIZE` until the frontend reports a measured one.
    /// Every show and resize is placed and clamped from this size.
    pub size: (f64, f64),
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            generation: 0,
            current: None,
            session: None,
            cursor: Point { x: 0, y: 0 },
            monitors: Vec::new(),
            size: INITIAL_POPUP_SIZE,
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
            image: None,
            last_model: None,
            replace_disabled: false,
        };
        let debug = format!("{s:?}");
        assert!(!debug.contains("PRIVATE-"));
        let with_image = Session {
            image: Some(Arc::new(
                CapturedImage::new(1, 1, vec![171, 205, 239, 255]).unwrap(),
            )),
            ..s
        };
        let debug = format!("{with_image:?}");
        assert!(debug.contains("1x1") && !debug.contains("171"), "{debug}");
    }
}
