//! `PlatformGate`: the single choke point for blocking platform calls.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use lingopop_platform::PlatformError;

/// How long a caller waits for the gate and for the job itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateTimeouts {
    /// Longest wait for the gate to become free before `GateError::Busy`.
    pub acquire: Duration,
    /// Longest wait for the job to return before `GateError::TimedOut`.
    pub run: Duration,
}

impl Default for GateTimeouts {
    fn default() -> Self {
        Self {
            acquire: Duration::from_secs(2),
            run: Duration::from_secs(3),
        }
    }
}

/// Why a gated call did not produce a value. No variant carries user data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    /// Another call held the gate for longer than `GateTimeouts::acquire`.
    #[error("platform is busy")]
    Busy,
    /// The job did not return within `GateTimeouts::run`.
    #[error("platform call timed out")]
    TimedOut,
    /// The job panicked, or its task was cancelled by a runtime shutdown.
    #[error("platform call panicked")]
    Panicked,
    /// The job itself failed.
    #[error("{0}")]
    Platform(PlatformError),
}

impl From<PlatformError> for GateError {
    fn from(e: PlatformError) -> Self {
        GateError::Platform(e)
    }
}

/// Serializes every blocking platform call of the app (clipboard, keys, foreground, monitors,
/// capture, OCR): at most one runs at a time and none runs on an async worker or the UI thread.
///
/// Semantics of [`PlatformGate::run`]:
/// 1. The gate is acquired within `acquire`, else `Busy`.
/// 2. The owned guard moves INTO the `spawn_blocking` closure, so the gate stays held until the
///    blocking job really returns, even when the caller has already given up.
/// 3. The job is awaited for `run`, else `TimedOut`. The job keeps running and keeps the gate;
///    later callers get `Busy` until it finishes.
/// 4. A panic gives `Panicked` and frees the gate (the guard drops while unwinding). A cancelled
///    task (runtime shutdown) is reported as `Panicked` too.
/// 5. A job error `e` gives `Platform(e)`.
#[derive(Clone)]
pub struct PlatformGate {
    lock: Arc<tokio::sync::Mutex<()>>,
    timeouts: GateTimeouts,
}

impl PlatformGate {
    pub fn new(timeouts: GateTimeouts) -> Self {
        Self {
            lock: Arc::new(tokio::sync::Mutex::new(())),
            timeouts,
        }
    }

    pub fn timeouts(&self) -> GateTimeouts {
        self.timeouts
    }

    /// Runs `job` on the blocking pool under the gate. See the type docs for the semantics.
    pub async fn run<T, F>(&self, job: F) -> Result<T, GateError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, PlatformError> + Send + 'static,
    {
        let guard = tokio::time::timeout(self.timeouts.acquire, self.lock.clone().lock_owned())
            .await
            .map_err(|_| GateError::Busy)?;
        let handle = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            job()
        });
        match tokio::time::timeout(self.timeouts.run, handle).await {
            Err(_) => Err(GateError::TimedOut),
            Ok(Err(_join_error)) => Err(GateError::Panicked),
            Ok(Ok(result)) => result.map_err(GateError::Platform),
        }
    }
}

impl fmt::Debug for PlatformGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformGate")
            .field("timeouts", &self.timeouts)
            .finish()
    }
}
