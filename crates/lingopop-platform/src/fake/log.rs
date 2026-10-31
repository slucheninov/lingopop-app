//! One ordered event log shared by the fakes, stamped with virtual time.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::clipboard::ClipboardCall;
use super::keys::KeyCall;
use super::sleeper::FakeSleeper;
use crate::foreground::WindowHandle;
use crate::sleeper::Sleeper;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Clipboard(ClipboardCall),
    Key(KeyCall),
    /// `ForegroundWindow::restore` was called for this window.
    Foreground(WindowHandle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedEvent {
    pub at: Duration,
    pub event: Event,
}

#[derive(Debug)]
pub struct EventLog {
    sleeper: Arc<FakeSleeper>,
    events: Mutex<Vec<LoggedEvent>>,
}

impl EventLog {
    pub fn new(sleeper: Arc<FakeSleeper>) -> Arc<Self> {
        Arc::new(Self {
            sleeper,
            events: Mutex::new(Vec::new()),
        })
    }

    pub fn record(&self, event: Event) {
        let at = self.sleeper.now();
        self.lock().push(LoggedEvent { at, event });
    }

    pub fn events(&self) -> Vec<LoggedEvent> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<LoggedEvent>> {
        self.events.lock().unwrap_or_else(|p| p.into_inner())
    }
}
