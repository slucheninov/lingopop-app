//! Scriptable foreground window.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use super::log::{Event, EventLog};
use crate::error::PlatformError;
use crate::foreground::{ForegroundWindow, WindowHandle};

struct State {
    current: Option<WindowHandle>,
    elevated: HashSet<WindowHandle>,
    fail_restore: bool,
    after_polls: u32,
    /// A successful restore waiting for enough `current()` polls to take effect.
    pending: Option<(WindowHandle, u32)>,
    restored: Vec<WindowHandle>,
}

pub struct FakeForeground {
    state: Mutex<State>,
    log: Mutex<Option<Arc<EventLog>>>,
}

impl FakeForeground {
    pub fn new(current: Option<WindowHandle>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                current,
                elevated: HashSet::new(),
                fail_restore: false,
                after_polls: 1,
                pending: None,
                restored: Vec::new(),
            }),
            log: Mutex::new(None),
        })
    }

    /// Records `restore` calls into a shared log (see `FakeClipboard::log`).
    pub fn log_to(&self, log: Arc<EventLog>) {
        *self.log.lock().unwrap_or_else(|p| p.into_inner()) = Some(log);
    }

    pub fn set_elevated(&self, window: WindowHandle, elevated: bool) {
        let mut s = self.state();
        if elevated {
            s.elevated.insert(window);
        } else {
            s.elevated.remove(&window);
        }
    }

    pub fn fail_restore(&self, fail: bool) {
        self.state().fail_restore = fail;
    }

    /// After `restore`, `current()` reports the target from its `n`-th call on (default 1).
    pub fn foreground_after_polls(&self, n: u32) {
        self.state().after_polls = n.max(1);
    }

    pub fn restored(&self) -> Vec<WindowHandle> {
        self.state().restored.clone()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl ForegroundWindow for FakeForeground {
    fn current(&self) -> Option<WindowHandle> {
        let mut s = self.state();
        if let Some((w, polls)) = s.pending {
            let polls = polls + 1;
            if polls >= s.after_polls {
                s.current = Some(w);
                s.pending = None;
            } else {
                s.pending = Some((w, polls));
            }
        }
        s.current
    }

    fn restore(&self, window: WindowHandle) -> Result<(), PlatformError> {
        if let Some(l) = self.log.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            l.record(Event::Foreground(window));
        }
        let mut s = self.state();
        s.restored.push(window);
        if s.fail_restore {
            return Err(PlatformError::Os {
                api: "SetForegroundWindow",
                code: 0,
            });
        }
        s.pending = Some((window, 0));
        Ok(())
    }

    fn is_elevated(&self, window: WindowHandle) -> bool {
        self.state().elevated.contains(&window)
    }
}
