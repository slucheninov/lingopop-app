//! The platform `MainThread` dispatcher over Tauri's event loop (macOS only).
//!
//! `post` hands the job to `AppHandle::run_on_main_thread` and returns at once; a caller that
//! needs the result waits for it with `lingopop_platform::main_thread::run_on_main` on its OWN
//! (worker) thread, outside every orchestrator, session and commit lock. On the main thread
//! that helper runs the job inline, so the main thread never waits on itself. When the event
//! loop is gone Tauri drops the job unrun, which releases such a waiter with a fixed error.
use lingopop_platform::{MainThread, PlatformError};

pub struct TauriMacMainThread(pub tauri::AppHandle);

impl MainThread for TauriMacMainThread {
    fn is_main_thread(&self) -> bool {
        lingopop_platform::main_thread::on_main_thread()
    }

    fn post(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), PlatformError> {
        self.0.run_on_main_thread(job).map_err(|_| {
            log::warn!("main-thread job could not be posted");
            lingopop_platform::main_thread::dispatch_error()
        })
    }
}
