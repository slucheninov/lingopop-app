//! A platform-local main-thread dispatcher seam (no Tauri here: the shell implements it).
//!
//! AppKit windows and `NSEvent` monitors must be touched on the main thread. Workers hand
//! that work to a [`MainThread`] and, when they need the result, wait for it OFF every lock
//! with [`run_on_main`]. Code already on the main thread runs inline, so the main thread
//! never waits on itself.
use crate::PlatformError;

/// Posts jobs to the process main thread (the Tauri / AppKit event loop).
pub trait MainThread: Send + Sync {
    /// True only on the process main thread.
    fn is_main_thread(&self) -> bool;
    /// Schedules `job` on the main thread and returns at once; it never runs `job` inline and
    /// never waits. A job that can no longer run (the loop is gone) is dropped unrun.
    fn post(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), PlatformError>;
}

/// The fixed error of a job that could not be posted or was dropped unrun.
pub fn dispatch_error() -> PlatformError {
    PlatformError::Os {
        api: "main thread dispatch",
        code: 0,
    }
}

/// Runs `job` on the main thread and returns its value: inline when already there, else
/// posted and waited for on the calling thread. The caller must hold no lock that main-thread
/// work could need; a dropped job returns [`dispatch_error`] instead of hanging.
pub fn run_on_main<T: Send + 'static>(
    main: &dyn MainThread,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, PlatformError> {
    if main.is_main_thread() {
        return Ok(job());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    main.post(Box::new(move || {
        let _ = tx.send(job());
    }))?;
    rx.recv().map_err(|_| dispatch_error())
}

/// Fire and forget: inline when already on the main thread, else posted without waiting.
pub fn post_to_main(
    main: &dyn MainThread,
    job: Box<dyn FnOnce() + Send>,
) -> Result<(), PlatformError> {
    if main.is_main_thread() {
        job();
        Ok(())
    } else {
        main.post(job)
    }
}

/// True on the process main thread (macOS: `pthread_main_np` through objc2). Always false
/// elsewhere, where no caller needs it.
pub fn on_main_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::on_main_thread()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
