//! Scriptable foreground window.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use super::log::{Event, EventLog};
use super::permissions::FakePermissions;
use crate::error::PlatformError;
use crate::foreground::{ForegroundWindow, WindowHandle};
use crate::permissions::{PermissionKind, PermissionState};

struct State {
    current: Option<WindowHandle>,
    elevated: HashSet<WindowHandle>,
    fail_restore: bool,
    after_polls: u32,
    /// A successful restore waiting for enough `current()` polls to take effect.
    pending: Option<(WindowHandle, u32)>,
    restored: Vec<WindowHandle>,
    /// Targets whose registry entry is gone (like a macOS handle that was evicted or expired).
    retired: HashSet<WindowHandle>,
    /// The foreground moves to this window at the next `is_current_target` call, as if the
    /// user switched apps right after the restore succeeded.
    drift: Option<WindowHandle>,
    target_checks: Vec<WindowHandle>,
    permissions: Option<Arc<FakePermissions>>,
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
                retired: HashSet::new(),
                drift: None,
                target_checks: Vec::new(),
                permissions: None,
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

    /// Changes what `current()` reports, as if the user switched windows.
    pub fn set_current(&self, window: Option<WindowHandle>) {
        let mut s = self.state();
        s.current = window;
        s.pending = None;
    }

    pub fn fail_restore(&self, fail: bool) {
        self.state().fail_restore = fail;
    }

    /// After `restore`, `current()` reports the target from its `n`-th call on (default 1).
    pub fn foreground_after_polls(&self, n: u32) {
        self.state().after_polls = n.max(1);
    }

    /// Like macOS: while Accessibility is not `Granted`, `is_current_target` fails with
    /// `PermissionDenied(Accessibility)` (the call is still recorded).
    pub fn require_permission(&self, permissions: Arc<FakePermissions>) {
        self.state().permissions = Some(permissions);
    }

    /// `window` becomes a stale target: `is_current_target` answers `Ok(false)` for it.
    pub fn retire_target(&self, window: WindowHandle) {
        self.state().retired.insert(window);
    }

    /// The foreground becomes `window` the next time `is_current_target` is called (a switch
    /// between the restore and the keystroke).
    pub fn switch_foreground_on_next_target_check(&self, window: WindowHandle) {
        self.state().drift = Some(window);
    }

    /// Every `is_current_target` argument, in call order.
    pub fn target_checks(&self) -> Vec<WindowHandle> {
        self.state().target_checks.clone()
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

    fn is_current_target(&self, window: WindowHandle) -> Result<bool, PlatformError> {
        let mut s = self.state();
        s.target_checks.push(window);
        if let Some(p) = &s.permissions {
            if p.peek(PermissionKind::Accessibility) != PermissionState::Granted {
                return Err(PlatformError::PermissionDenied(
                    PermissionKind::Accessibility,
                ));
            }
        }
        if let Some(other) = s.drift.take() {
            s.current = Some(other);
            s.pending = None;
        }
        Ok(!s.retired.contains(&window) && s.current == Some(window))
    }
}
