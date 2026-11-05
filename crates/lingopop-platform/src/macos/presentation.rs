//! Portable half of the macOS popup presentation, compiled on every OS as
//! `lingopop_platform::macos_presentation` so its ordering contract runs on fakes.
//!
//! The shell's presentation worker translates a queued presentation into a
//! [`MacWindowCommand`] (native handle + desktop rect in Quartz points) and calls
//! [`MacPresenter::execute`], which runs the window effect on the main thread through the
//! injected [`MainThread`] and waits for it on the worker. The staleness check runs again ON
//! MAIN, right before the effect, so a Show that a dismissal superseded while it was queued
//! on the main loop never reaches AppKit. No Tauri or orchestrator dependency.
use crate::main_thread::{run_on_main, MainThread};
use crate::{NoActivateWindow, PlatformError, Rect, WindowHandle};
use std::sync::Arc;

/// One native popup window effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacWindowCommand {
    /// Place at `rect` (desktop units: Quartz points) and order front without activating.
    Show { window: WindowHandle, rect: Rect },
    /// Order out.
    Hide { window: WindowHandle },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacWindowOutcome {
    /// The effect ran to completion on the main thread.
    Applied,
    /// The command was no longer current when the main thread reached it; nothing ran.
    Stale,
}

/// "Is this command still the newest one?" Called on the main thread; must not block.
pub type CurrentCheck = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct MacPresenter {
    main: Arc<dyn MainThread>,
    window: Arc<dyn NoActivateWindow>,
}

impl MacPresenter {
    /// `window` is the main-thread-only native popup (`MacPopupWindow` in production).
    pub fn new(main: Arc<dyn MainThread>, window: Arc<dyn NoActivateWindow>) -> Self {
        Self { main, window }
    }

    /// Runs `command` on the main thread and returns only after it finished there (inline
    /// when called on main). The caller must hold no lock: this blocks the calling worker.
    pub fn execute(
        &self,
        command: MacWindowCommand,
        current: CurrentCheck,
    ) -> Result<MacWindowOutcome, PlatformError> {
        let window = self.window.clone();
        run_on_main(self.main.as_ref(), move || {
            if !current() {
                return Ok(MacWindowOutcome::Stale);
            }
            match command {
                MacWindowCommand::Show {
                    window: handle,
                    rect,
                } => window.show_at(handle, rect),
                MacWindowCommand::Hide { window: handle } => window.hide(handle),
            }
            .map(|()| MacWindowOutcome::Applied)
        })?
    }
}
