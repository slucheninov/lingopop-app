//! OS-agnostic orchestration over the `lingopop-platform` traits: the flow logic of the app
//! (capture, translate, replace) lives here, free of any UI framework. This crate never depends
//! on `tauri` or on an OS crate; the app crate wires the real platform in.
#![forbid(unsafe_code)]

pub mod gate;

pub use gate::{GateError, GateTimeouts, PlatformGate};
