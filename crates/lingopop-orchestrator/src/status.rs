//! Synchronous status delivery; implementations must not call back into the orchestrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Idle,
    Working,
    Error,
}
pub trait StatusSink: Send + Sync {
    fn status(&self, status: TrayStatus);
}
