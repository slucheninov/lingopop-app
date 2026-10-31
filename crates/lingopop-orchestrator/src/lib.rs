//! OS-agnostic orchestration over the `lingopop-platform` traits: the flow logic of the app
//! (capture, translate, replace) lives here, free of any UI framework. This crate never depends
//! on `tauri` or on an OS crate; the app crate wires the real platform in.
#![forbid(unsafe_code)]

pub mod fake;
pub mod gate;
pub mod messages;
pub mod orchestrator;
pub mod placement;
pub mod popup;
pub mod providers;
pub mod session;
pub mod view_model;

pub use gate::{GateError, GateTimeouts, PlatformGate};
pub use messages::{
    gate_message, max_input, missing_key, no_text_found, nothing_selected, platform_message,
    selection_too_small, translation_error_view,
};
pub use orchestrator::{ConfigSource, Deps, Orchestrator, Persist};
pub use placement::popup_origin;
pub use popup::{PopupView, INITIAL_POPUP_SIZE};
pub use providers::{ocr_translate_providers, retry_providers, vision_providers};
pub use view_model::{
    ActionRowView, ErrorView, FallbackView, PopupPhase, PopupViewModel, ResultView, RetryProvider,
    UserMessageView,
};
