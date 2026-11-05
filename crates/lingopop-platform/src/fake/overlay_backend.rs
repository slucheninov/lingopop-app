//! `FakeOverlayBackend`: a recording `OverlayBackend` for the portable macOS overlay tests.
//! Every create / cleanup gets a global sequence number, so tests can assert that the first
//! overlay's cleanup precedes the second one's creation.
use crate::macos_geometry::MacGeometry;
use crate::macos_overlay::{OverlayBackend, OverlaySink};
use crate::overlay::OverlayInput;
use crate::{PlatformError, Rect};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayCall {
    Create {
        seq: u64,
        displays: usize,
    },
    Draw(Option<Rect>),
    CleanupStarted {
        seq: u64,
    },
    Cleanup {
        seq: u64,
    },
    /// The previously frontmost application was handed focus back.
    RestorePreviousApp {
        seq: u64,
    },
}

#[derive(Default)]
struct State {
    calls: Vec<OverlayCall>,
    seq: u64,
    sink: Option<Arc<OverlaySink>>,
    live: usize,
    max_live: usize,
    hold_cleanup: bool,
    fail_create: Option<PlatformError>,
}

#[derive(Default)]
pub struct FakeOverlayBackend {
    state: Mutex<State>,
    changed: Condvar,
}

impl FakeOverlayBackend {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn wait_until(&self, mut done: impl FnMut(&State) -> bool) {
        let mut s = self.state();
        while !done(&s) {
            s = self.changed.wait(s).unwrap_or_else(|p| p.into_inner());
        }
    }
    pub fn calls(&self) -> Vec<OverlayCall> {
        self.state().calls.clone()
    }
    fn count(&self, f: impl Fn(&OverlayCall) -> bool) -> usize {
        self.state().calls.iter().filter(|c| f(c)).count()
    }
    pub fn creates(&self) -> usize {
        self.count(|c| matches!(c, OverlayCall::Create { .. }))
    }
    pub fn cleanups(&self) -> usize {
        self.count(|c| matches!(c, OverlayCall::Cleanup { .. }))
    }
    /// Every rendered selection, in order.
    pub fn draws(&self) -> Vec<Option<Rect>> {
        self.state()
            .calls
            .iter()
            .filter_map(|c| match c {
                OverlayCall::Draw(r) => Some(*r),
                _ => None,
            })
            .collect()
    }
    /// Sequence number of the `n`th (1-based) create.
    pub fn create_seq(&self, n: usize) -> Option<u64> {
        self.state()
            .calls
            .iter()
            .filter_map(|c| match c {
                OverlayCall::Create { seq, .. } => Some(*seq),
                _ => None,
            })
            .nth(n.checked_sub(1)?)
    }
    /// How many times the previous application was restored.
    pub fn restores(&self) -> usize {
        self.count(|c| matches!(c, OverlayCall::RestorePreviousApp { .. }))
    }
    /// Sequence number of the `n`th (1-based) restore of the previous application.
    pub fn restore_seq(&self, n: usize) -> Option<u64> {
        self.state()
            .calls
            .iter()
            .filter_map(|c| match c {
                OverlayCall::RestorePreviousApp { seq } => Some(*seq),
                _ => None,
            })
            .nth(n.checked_sub(1)?)
    }
    /// Sequence number of the `n`th (1-based) finished cleanup.
    pub fn cleanup_seq(&self, n: usize) -> Option<u64> {
        self.state()
            .calls
            .iter()
            .filter_map(|c| match c {
                OverlayCall::Cleanup { seq } => Some(*seq),
                _ => None,
            })
            .nth(n.checked_sub(1)?)
    }
    /// Displays the most recent create was asked to cover.
    pub fn last_display_count(&self) -> usize {
        self.state()
            .calls
            .iter()
            .rev()
            .find_map(|c| match c {
                OverlayCall::Create { displays, .. } => Some(*displays),
                _ => None,
            })
            .unwrap_or(0)
    }
    /// Native overlays alive right now.
    pub fn native_active(&self) -> usize {
        self.state().live
    }
    pub fn max_native(&self) -> usize {
        self.state().max_live
    }
    /// Cleanups block (after being recorded as started) until `hold_cleanup(false)`.
    pub fn hold_cleanup(&self, hold: bool) {
        self.state().hold_cleanup = hold;
        self.changed.notify_all();
    }
    /// The next create fails with `error`.
    pub fn fail_next_create(&self, error: PlatformError) {
        self.state().fail_create = Some(error);
    }
    pub fn wait_for_creates(&self, n: usize) {
        self.wait_until(|s| {
            s.calls
                .iter()
                .filter(|c| matches!(c, OverlayCall::Create { .. }))
                .count()
                >= n
        });
    }
    pub fn wait_for_cleanups_started(&self, n: usize) {
        self.wait_until(|s| {
            s.calls
                .iter()
                .filter(|c| matches!(c, OverlayCall::CleanupStarted { .. }))
                .count()
                >= n
        });
    }
    pub fn wait_for_cleanups(&self, n: usize) {
        self.wait_until(|s| {
            s.calls
                .iter()
                .filter(|c| matches!(c, OverlayCall::Cleanup { .. }))
                .count()
                >= n
        });
    }
    /// Delivers `input` to the live overlay like a native monitor; returns whether it was
    /// consumed (false without an overlay or after it is gone).
    pub fn fire(&self, input: OverlayInput) -> bool {
        let sink = self.state().sink.clone();
        sink.is_some_and(|sink| sink.push(input))
    }
}

impl OverlayBackend for FakeOverlayBackend {
    fn create(&self, geometry: &MacGeometry, sink: Arc<OverlaySink>) -> Result<(), PlatformError> {
        let mut s = self.state();
        s.seq += 1;
        let seq = s.seq;
        s.calls.push(OverlayCall::Create {
            seq,
            displays: geometry.displays().len(),
        });
        let failed = s.fail_create.take();
        if failed.is_none() {
            s.live += 1;
            s.max_live = s.max_live.max(s.live);
            s.sink = Some(sink);
        }
        drop(s);
        self.changed.notify_all();
        failed.map_or(Ok(()), Err)
    }

    fn draw(&self, selection: Option<Rect>) {
        self.state().calls.push(OverlayCall::Draw(selection));
    }

    fn cleanup(&self) {
        let mut s = self.state();
        s.seq += 1;
        let seq = s.seq;
        s.calls.push(OverlayCall::CleanupStarted { seq });
        self.changed.notify_all();
        while s.hold_cleanup {
            s = self.changed.wait(s).unwrap_or_else(|p| p.into_inner());
        }
        s.seq += 1;
        let seq = s.seq;
        if s.sink.take().is_some() {
            s.live = s.live.saturating_sub(1);
        }
        s.calls.push(OverlayCall::Cleanup { seq });
        drop(s);
        self.changed.notify_all();
    }

    fn restore_previous_app(&self) {
        let mut s = self.state();
        s.seq += 1;
        let seq = s.seq;
        s.calls.push(OverlayCall::RestorePreviousApp { seq });
        drop(s);
        self.changed.notify_all();
    }
}
