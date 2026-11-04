//! A scriptable permission service: per-kind state, a scripted outcome for `request`, and
//! counters, so tests can assert that nothing prompted.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::PlatformError;
use crate::permissions::{PermissionKind, PermissionState, Permissions};

#[derive(Default)]
struct State {
    states: BTreeMap<PermissionKind, PermissionState>,
    outcomes: BTreeMap<PermissionKind, PermissionState>,
    requests: Vec<PermissionKind>,
    opened: Vec<PermissionKind>,
    reads: usize,
    fail_open: bool,
}

/// Every kind starts `Granted`. `request` records the call and, when an outcome was scripted for
/// the kind, moves the state to it; otherwise the state is unchanged.
#[derive(Default)]
pub struct FakePermissions {
    state: Mutex<State>,
}

impl FakePermissions {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_state(&self, kind: PermissionKind, state: PermissionState) {
        self.lock().states.insert(kind, state);
    }

    /// What `state(kind)` becomes when `request(kind)` runs.
    pub fn set_request_outcome(&self, kind: PermissionKind, state: PermissionState) {
        self.lock().outcomes.insert(kind, state);
    }

    /// Makes `open_settings` fail with an `Os` error.
    pub fn fail_open_settings(&self) {
        self.lock().fail_open = true;
    }

    pub fn request_calls(&self, kind: PermissionKind) -> usize {
        self.lock().requests.iter().filter(|k| **k == kind).count()
    }

    /// Every `request` call in order.
    pub fn requests(&self) -> Vec<PermissionKind> {
        self.lock().requests.clone()
    }

    /// Every successful `open_settings` call in order.
    pub fn opened(&self) -> Vec<PermissionKind> {
        self.lock().opened.clone()
    }

    /// How many times `state` was read.
    pub fn state_reads(&self) -> usize {
        self.lock().reads
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn current(state: &State, kind: PermissionKind) -> PermissionState {
        state
            .states
            .get(&kind)
            .copied()
            .unwrap_or(PermissionState::Granted)
    }

    /// For the permission-aware fakes: `state` without counting a read.
    pub(crate) fn peek(&self, kind: PermissionKind) -> PermissionState {
        Self::current(&self.lock(), kind)
    }
}

impl Permissions for FakePermissions {
    fn state(&self, kind: PermissionKind) -> PermissionState {
        let mut s = self.lock();
        s.reads += 1;
        Self::current(&s, kind)
    }

    fn request(&self, kind: PermissionKind) -> Result<PermissionState, PlatformError> {
        let mut s = self.lock();
        s.requests.push(kind);
        if let Some(outcome) = s.outcomes.get(&kind).copied() {
            s.states.insert(kind, outcome);
        }
        Ok(Self::current(&s, kind))
    }

    fn open_settings(&self, kind: PermissionKind) -> Result<(), PlatformError> {
        let mut s = self.lock();
        if s.fail_open {
            return Err(PlatformError::Os {
                api: "open",
                code: 1,
            });
        }
        s.opened.push(kind);
        Ok(())
    }
}
