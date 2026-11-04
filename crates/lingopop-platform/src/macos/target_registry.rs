//! Exact source targeting for macOS: a bounded registry of captured applications and the
//! `ForegroundWindow` / last-external logic over it.
//!
//! Compiled on every OS (the `macos_targets` module, like `macos_geometry`) so the whole decision
//! logic is unit-tested without AppKit. It has no `unsafe`; the AppKit and Accessibility calls
//! sit behind the [`MacApps`] seam and are implemented in `foreground.rs` / `ax.rs`.
//!
//! # Handles
//! `WindowHandle(u64)` stays opaque. On macOS it packs a registry slot (low 8 bits) and a
//! generation (the rest). The generation grows with every new entry, so a handle whose slot was
//! evicted, expired or reused is rejected instead of silently naming another application.
//!
//! # What an entry pins
//! The process (pid + bundle id + launch date, so a reused pid is a different process) and, where
//! Accessibility allows it, the focused AX window at capture time. The AX window is best effort:
//! it is used to raise the captured window before the application is activated, never to refuse
//! a paste. The rule is: refuse only when the frontmost process is not the captured one, or the
//! entry is gone. Another window of the SAME process cannot be told apart without AX and is a
//! documented limit, not a guarantee.
//!
//! # Not atomic
//! [`TargetTracker::is_current_target`] is a look, not a lock: the user can switch apps right
//! after it answers. It narrows the race before a keystroke, it does not close it.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::error::PlatformError;
use crate::foreground::WindowHandle;
use crate::permissions::PermissionKind;
use crate::sleeper::Sleeper;

/// Registry capacity. Beyond it the entry seen longest ago is evicted.
pub const CAPACITY: usize = 32;
/// An entry that was not observed in the foreground for this long is expired.
pub const ENTRY_LIFETIME: Duration = Duration::from_secs(10 * 60);

const INDEX_BITS: u32 = 8;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
const _: () = assert!(CAPACITY as u64 <= INDEX_MASK + 1);

/// Who an application is, independent of its pid being reused later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    pub pid: i32,
    pub bundle_id: Option<String>,
    /// Launch time in milliseconds since the Apple reference date.
    pub launch: Option<i64>,
}

impl AppIdentity {
    /// The same running process: equal pid, and bundle id / launch date equal wherever both
    /// sides know them (an unknown value on one side is not a mismatch). A pid reused by another
    /// launch differs in `launch` (or `bundle_id`).
    pub fn same_process(&self, other: &AppIdentity) -> bool {
        fn compatible<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
        }
        self.pid == other.pid
            && compatible(&self.bundle_id, &other.bundle_id)
            && compatible(&self.launch, &other.launch)
    }
}

/// The operating-system seam: everything the targeting logic asks of macOS.
pub trait MacApps: Send + Sync {
    /// Handle to an AX window element. Compared with `==` (CFEqual) to tell windows of one
    /// process apart; never shown or logged.
    type Ax: Clone + PartialEq + Send + Sync;
    /// `AXIsProcessTrusted()`. Never prompts.
    fn accessibility_trusted(&self) -> bool;
    /// The frontmost application, if any.
    fn frontmost(&self) -> Option<AppIdentity>;
    /// The running, not terminated application with this pid.
    fn running(&self, pid: i32) -> Option<AppIdentity>;
    /// The focused AX window of the application, best effort (`None` = unknown).
    fn focused_window(&self, pid: i32) -> Option<Self::Ax>;
    /// Raise `window` when given (best effort), then activate the application.
    fn activate(&self, pid: i32, window: Option<&Self::Ax>) -> bool;
}

/// A resolved registry entry.
#[derive(Debug, Clone)]
pub struct Target<A> {
    pub identity: AppIdentity,
    pub ax: Option<A>,
}

struct Slot<A> {
    generation: u64,
    last_seen: Duration,
    target: Target<A>,
}

/// Fixed-capacity, generation-stamped registry.
pub struct TargetRegistry<A> {
    slots: Vec<Option<Slot<A>>>,
    next_generation: u64,
}

fn pack(index: usize, generation: u64) -> WindowHandle {
    WindowHandle((generation << INDEX_BITS) | index as u64)
}

fn unpack(handle: WindowHandle) -> (usize, u64) {
    ((handle.0 & INDEX_MASK) as usize, handle.0 >> INDEX_BITS)
}

impl<A: Clone + PartialEq> Default for TargetRegistry<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone + PartialEq> TargetRegistry<A> {
    pub fn new() -> Self {
        Self {
            slots: (0..CAPACITY).map(|_| None).collect(),
            next_generation: 1,
        }
    }

    /// Live (unexpired as of the last call) entries.
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn expire(&mut self, now: Duration) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|s| now.saturating_sub(s.last_seen) >= ENTRY_LIFETIME)
            {
                *slot = None;
            }
        }
    }

    /// Records an observation of `identity` (with its focused AX window when known) and returns
    /// the handle. The same process and a compatible window (equal AX windows, or one side
    /// unknown) keeps its handle, so `current()` is stable between polls; the observation
    /// refreshes the entry's lifetime. A different window or a new launch gets a new entry. Full
    /// registry: the entry seen longest ago is evicted.
    pub fn observe(&mut self, now: Duration, identity: AppIdentity, ax: Option<A>) -> WindowHandle {
        self.expire(now);
        let same = |s: &Slot<A>| {
            s.target.identity == identity
                && match (&s.target.ax, &ax) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                }
        };
        if let Some(index) = self.slots.iter().position(|s| s.as_ref().is_some_and(same)) {
            let slot = self.slots[index].as_mut().expect("position found it");
            slot.last_seen = now;
            if slot.target.ax.is_none() {
                slot.target.ax = ax;
            }
            return pack(index, slot.generation);
        }
        let index = self
            .slots
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.slots
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| {
                        s.as_ref()
                            .map(|s| (s.last_seen, s.generation))
                            .unwrap_or_default()
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        let generation = self.next_generation;
        self.next_generation += 1;
        self.slots[index] = Some(Slot {
            generation,
            last_seen: now,
            target: Target { identity, ax },
        });
        pack(index, generation)
    }

    /// The entry a handle names. Unknown slot, wrong generation (evicted / reused) or expired =
    /// `None`. Does not extend the lifetime.
    pub fn get(&mut self, now: Duration, handle: WindowHandle) -> Option<Target<A>> {
        self.expire(now);
        let (index, generation) = unpack(handle);
        self.slots
            .get(index)?
            .as_ref()
            .filter(|s| s.generation == generation)
            .map(|s| s.target.clone())
    }
}

fn target_lost() -> PlatformError {
    // Fixed text, no application or window data.
    PlatformError::Os {
        api: "NSRunningApplication.activate",
        code: 0,
    }
}

/// `current` / `restore` / `is_current_target` over a [`MacApps`] and a registry.
pub struct TargetTracker<P: MacApps> {
    apps: P,
    clock: Arc<dyn Sleeper>,
    registry: Mutex<TargetRegistry<P::Ax>>,
}

impl<P: MacApps> TargetTracker<P> {
    pub fn new(apps: P, clock: Arc<dyn Sleeper>) -> Self {
        Self {
            apps,
            clock,
            registry: Mutex::new(TargetRegistry::new()),
        }
    }

    fn registry(&self) -> MutexGuard<'_, TargetRegistry<P::Ax>> {
        self.registry.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Entries currently held (tests).
    pub fn registered(&self) -> usize {
        self.registry().len()
    }

    /// The frontmost application as a registered handle plus its identity, with its focused AX
    /// window when Accessibility is trusted.
    pub fn observe_frontmost(&self) -> Option<(WindowHandle, AppIdentity)> {
        self.observe_frontmost_with(true)
    }

    /// Like `observe_frontmost`; `with_ax = false` never asks an application for its window
    /// (the background poller: no IPC into other processes four times a second).
    pub fn observe_frontmost_with(&self, with_ax: bool) -> Option<(WindowHandle, AppIdentity)> {
        let front = self.apps.frontmost().filter(|f| f.pid > 0)?;
        // AX calls are IPC to another process: never under the registry lock.
        let ax = if with_ax && self.apps.accessibility_trusted() {
            self.apps.focused_window(front.pid)
        } else {
            None
        };
        let handle = self.registry().observe(self.clock.now(), front.clone(), ax);
        Some((handle, front))
    }

    pub fn current(&self) -> Option<WindowHandle> {
        self.observe_frontmost().map(|(h, _)| h)
    }

    /// The entry names a process that is still the same running process.
    pub fn is_live(&self, handle: WindowHandle) -> bool {
        let Some(target) = self.registry().get(self.clock.now(), handle) else {
            return false;
        };
        self.apps
            .running(target.identity.pid)
            .is_some_and(|live| live.same_process(&target.identity))
    }

    /// Brings the captured application (and, when known, its captured window) to the front.
    /// An evicted / expired / unknown handle, an exited process or a reused pid is an error.
    pub fn restore(&self, handle: WindowHandle) -> Result<(), PlatformError> {
        let target = self
            .registry()
            .get(self.clock.now(), handle)
            .ok_or_else(target_lost)?;
        let live = self
            .apps
            .running(target.identity.pid)
            .filter(|live| live.same_process(&target.identity))
            .ok_or_else(target_lost)?;
        // The raise needs Accessibility; without it only the application is activated.
        let window = if self.apps.accessibility_trusted() {
            target.ax.as_ref()
        } else {
            None
        };
        if self.apps.activate(live.pid, window) {
            Ok(())
        } else {
            Err(target_lost())
        }
    }

    /// See the module docs: NOT atomic. `Err(PermissionDenied(Accessibility))` when Accessibility
    /// was revoked (distinct from target loss); `Ok(false)` for a different frontmost process, a
    /// reused pid, an evicted / expired / unknown handle. A missing AX window never refuses.
    pub fn is_current_target(&self, handle: WindowHandle) -> Result<bool, PlatformError> {
        if !self.apps.accessibility_trusted() {
            return Err(PlatformError::PermissionDenied(
                PermissionKind::Accessibility,
            ));
        }
        let Some(target) = self.registry().get(self.clock.now(), handle) else {
            return Ok(false);
        };
        Ok(self
            .apps
            .frontmost()
            .is_some_and(|front| front.same_process(&target.identity)))
    }
}

/// Remembers the last foreground application that is not ours.
pub struct LastExternalTracker<P: MacApps> {
    tracker: Arc<TargetTracker<P>>,
    own_pid: i32,
    last: Mutex<Option<WindowHandle>>,
}

impl<P: MacApps> LastExternalTracker<P> {
    pub fn new(tracker: Arc<TargetTracker<P>>, own_pid: i32) -> Self {
        Self {
            tracker,
            own_pid,
            last: Mutex::new(None),
        }
    }

    /// Samples the foreground, asking the application for its focused window; our own
    /// application (any window of it) never replaces the remembered one.
    pub fn observe(&self) {
        self.observe_with(true);
    }

    /// The cheap sample for a background poller: identity only, no Accessibility IPC.
    pub fn observe_light(&self) {
        self.observe_with(false);
    }

    fn observe_with(&self, with_ax: bool) {
        if let Some((handle, front)) = self.tracker.observe_frontmost_with(with_ax) {
            if front.pid != self.own_pid {
                *self.last.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
            }
        }
    }

    /// The remembered application, sampled just now, while it still runs.
    pub fn last_external(&self) -> Option<WindowHandle> {
        self.observe();
        let handle = (*self.last.lock().unwrap_or_else(|p| p.into_inner()))?;
        self.tracker.is_live(handle).then_some(handle)
    }
}

impl<P: MacApps> crate::last_foreground::LastForeground for LastExternalTracker<P> {
    fn last_external(&self) -> Option<WindowHandle> {
        LastExternalTracker::last_external(self)
    }
}

/// A scriptable [`MacApps`] for tests (always compiled, like the other fakes).
pub struct FakeApps {
    state: Mutex<FakeAppsState>,
}

#[derive(Default)]
struct FakeAppsState {
    trusted: bool,
    frontmost: Option<AppIdentity>,
    running: Vec<AppIdentity>,
    focused: Vec<(i32, u32)>,
    activated: Vec<(i32, Option<u32>)>,
    fail_activate: bool,
}

impl FakeApps {
    /// Accessibility trusted, nothing running.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeAppsState {
                trusted: true,
                ..Default::default()
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, FakeAppsState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_trusted(&self, trusted: bool) {
        self.state().trusted = trusted;
    }

    /// Starts `app` (replacing a running one with the same pid, like a pid reuse).
    pub fn launch(&self, app: AppIdentity) {
        let mut s = self.state();
        s.running.retain(|r| r.pid != app.pid);
        s.running.push(app);
    }

    pub fn quit(&self, pid: i32) {
        let mut s = self.state();
        s.running.retain(|r| r.pid != pid);
        if s.frontmost.as_ref().is_some_and(|f| f.pid == pid) {
            s.frontmost = None;
        }
    }

    /// `app` becomes the frontmost application (it is launched when not running yet).
    pub fn set_frontmost(&self, app: AppIdentity) {
        self.launch(app.clone());
        self.state().frontmost = Some(app);
    }

    /// The AX window id of `pid`'s focused window (`None` pid entry = AX unknown).
    pub fn set_focused_window(&self, pid: i32, window: u32) {
        let mut s = self.state();
        s.focused.retain(|(p, _)| *p != pid);
        s.focused.push((pid, window));
    }

    pub fn clear_focused_window(&self, pid: i32) {
        self.state().focused.retain(|(p, _)| *p != pid);
    }

    pub fn fail_activate(&self, fail: bool) {
        self.state().fail_activate = fail;
    }

    /// `(pid, raised window)` per `activate` call.
    pub fn activated(&self) -> Vec<(i32, Option<u32>)> {
        self.state().activated.clone()
    }
}

impl MacApps for Arc<FakeApps> {
    type Ax = u32;

    fn accessibility_trusted(&self) -> bool {
        self.state().trusted
    }

    fn frontmost(&self) -> Option<AppIdentity> {
        self.state().frontmost.clone()
    }

    fn running(&self, pid: i32) -> Option<AppIdentity> {
        self.state().running.iter().find(|r| r.pid == pid).cloned()
    }

    fn focused_window(&self, pid: i32) -> Option<u32> {
        self.state()
            .focused
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, w)| *w)
    }

    fn activate(&self, pid: i32, window: Option<&u32>) -> bool {
        let mut s = self.state();
        s.activated.push((pid, window.copied()));
        if s.fail_activate {
            return false;
        }
        if let Some(app) = s.running.iter().find(|r| r.pid == pid).cloned() {
            s.frontmost = Some(app);
        }
        true
    }
}
