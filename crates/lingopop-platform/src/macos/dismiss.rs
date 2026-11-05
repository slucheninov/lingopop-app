//! Portable half of the macOS popup dismissal, compiled on every OS as
//! `lingopop_platform::macos_dismiss` so the whole decision logic runs on fakes.
//!
//! The popup never becomes key, so dismissal is observed with PASSIVE `NSEvent` monitors
//! (`macos/event_monitors.rs`): global + local mouse-down for a click outside the popup, and
//! a keyDown monitor for Escape. A global keyDown monitor only sees other apps' keys with
//! Accessibility trust; without it Escape is unavailable ([`MacDismissWatcher::escape_available`])
//! while click-outside keeps working. Nothing is ever swallowed: the local handler returns
//! the event unchanged, global monitors cannot consume, and Escape still reaches the app the
//! user is typing in (a documented macOS difference from Windows).
//!
//! Every `DismissWatcher` method except `shutdown` is non-blocking (the orchestrator calls
//! them under its session mutex): geometry and the active flag are published at once and
//! installation / removal is posted to the main thread as one idempotent reconcile job, which
//! installs the set the CURRENT trust allows and reinstalls when it differs from the installed
//! one (a trust change applies on the next start). Escape availability is read from the
//! INSTALLED set. Monitor callbacks only filter and `try_send` into a bounded channel.
//!
//! `shutdown` is deterministic: on the main thread it removes every monitor inline, even with
//! a reconcile already queued; off the main thread it posts the removal and waits for it (a
//! completion barrier, bounded by [`SHUTDOWN_CLEANUP_TIMEOUT`]), so a caller that resolves its
//! own completion afterwards (the presentation worker) does so only after native removal.
use crate::error::platform_log;
use crate::main_thread::{post_to_main, MainThread};
use crate::{DismissEvent, DismissWatcher, PlatformError, Point, Rect};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Upper bound for an off-main `shutdown` waiting on the main thread's monitor removal. The
/// wait ends earlier when the job runs, or at once when the main loop drops it unrun.
pub const SHUTDOWN_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// `kCGEventSourceUserData` value stamped on every CGEvent `MacKeys` posts ("LPOP"), so our
/// own synthesized Cmd+C / Cmd+V never dismiss the popup. Foreign injected input still counts.
pub const OWN_EVENT_MARKER: i64 = 0x4C50_4F50;
/// `kVK_Escape`.
pub const ESCAPE_KEY_CODE: u16 = 53;

/// "A region-selection overlay is up": shared between the overlay engine (takes one
/// [`OverlayLease`] per pick before the native overlay exists, releases it only after that
/// pick's cleanup ran) and the dismiss sink (drops every event while any lease is held). The
/// overlay's own drag-start click and Escape are events to our own app, so the dismiss
/// watcher's LOCAL monitors see them too, and AppKit does not document the order of local
/// monitors: relying on it could make a Screen Translate started with a popup open cancel
/// itself on its first click.
///
/// Ownership is per pick, not a single boolean: a cleanup that timed out stays queued on the
/// main thread and may run after the next pick started, and it must only give back ITS lease.
#[derive(Debug, Clone, Default)]
pub struct OverlayActive(Arc<AtomicUsize>);

impl OverlayActive {
    pub fn new() -> Self {
        Self::default()
    }
    /// Takes a lease for one pick; the flag reads set until every lease was released.
    pub fn begin(&self) -> OverlayLease {
        self.0.fetch_add(1, Ordering::SeqCst);
        OverlayLease {
            count: self.0.clone(),
            released: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst) > 0
    }
}

/// One pick's claim on [`OverlayActive`]. Clones share the claim and [`release`](Self::release)
/// is idempotent across all of them, so every exit path of a pick may release without
/// counting; dropping a lease does NOT release it (a posted job that never ran is released
/// explicitly by the code that knows it never will).
#[derive(Debug, Clone)]
pub struct OverlayLease {
    count: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
}

impl OverlayLease {
    pub fn release(&self) {
        if !self.released.swap(true, Ordering::SeqCst) {
            self.count.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// What a native monitor saw, reduced to the fields the decision needs (no characters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedEvent {
    /// Any mouse button down; `location` in desktop units (Quartz global points).
    MouseDown { location: Point, marker: i64 },
    /// `modified` = Shift, Control, Option or Command held.
    KeyDown {
        key_code: u16,
        modified: bool,
        marker: i64,
    },
}

/// Which monitors one installation consists of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorSet {
    pub global_mouse: bool,
    pub local_mouse: bool,
    /// Needs Accessibility trust (other apps' keystrokes).
    pub global_key: bool,
    pub local_key: bool,
}

impl MonitorSet {
    pub fn for_trust(trusted: bool) -> Self {
        Self {
            global_mouse: true,
            local_mouse: true,
            global_key: trusted,
            local_key: true,
        }
    }
    pub fn count(&self) -> usize {
        [
            self.global_mouse,
            self.local_mouse,
            self.global_key,
            self.local_key,
        ]
        .iter()
        .filter(|on| **on)
        .count()
    }
}

/// Native monitor installation. `install` / `remove_all` are called on the main thread only.
pub trait MonitorBackend: Send + Sync + 'static {
    /// Accessibility trust right now (never prompts).
    fn trusted(&self) -> bool;
    /// Installs `set`, every callback feeding `sink`. On `Err` the watcher calls `remove_all`.
    fn install(&self, set: MonitorSet, sink: Arc<DismissSink>) -> Result<(), PlatformError>;
    /// Removes every monitor this backend installed; idempotent.
    fn remove_all(&self);
}

/// The callback side: filters an observed event and emits at most one dismissal per start.
pub struct DismissSink {
    active: AtomicBool,
    overlay: OverlayActive,
    rect: Mutex<Rect>,
    sender: crossbeam_channel::Sender<DismissEvent>,
}

impl DismissSink {
    fn rect(&self) -> MutexGuard<'_, Rect> {
        self.rect.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Called from monitor callbacks (main thread). Never blocks beyond the rect read.
    pub fn observe(&self, event: ObservedEvent) {
        // While a region overlay is up every dismiss event (mouse and Escape, local and global)
        // belongs to the overlay; the watcher stays armed and resumes afterwards.
        if !self.active.load(Ordering::SeqCst) || self.overlay.is_set() {
            return;
        }
        let dismissal = match event {
            ObservedEvent::MouseDown { marker, .. } | ObservedEvent::KeyDown { marker, .. }
                if marker == OWN_EVENT_MARKER =>
            {
                None
            }
            ObservedEvent::MouseDown { location, .. } => {
                crate::classify_click(location, *self.rect())
            }
            ObservedEvent::KeyDown {
                key_code, modified, ..
            } => (key_code == ESCAPE_KEY_CODE && !modified).then_some(DismissEvent::Escape),
        };
        if let Some(event) = dismissal {
            if self.active.swap(false, Ordering::SeqCst) {
                let _ = self.sender.try_send(event);
            }
        }
    }
}

#[derive(Default)]
struct Control {
    /// The orchestrator wants monitors (between start and stop).
    wanted: bool,
    /// Process teardown ran; nothing is installed again.
    sealed: bool,
    /// A reconcile job is queued on the main thread.
    posted: bool,
    /// The monitor set installed right now (only main-thread work changes it).
    installed: Option<MonitorSet>,
    /// The sealed removal ran on the main thread: nothing is installed, ever again.
    cleaned: bool,
}

struct Inner<B: MonitorBackend> {
    backend: Arc<B>,
    sink: Arc<DismissSink>,
    receiver: crossbeam_channel::Receiver<DismissEvent>,
    main: OnceLock<Arc<dyn MainThread>>,
    control: Mutex<Control>,
}

impl<B: MonitorBackend> Inner<B> {
    fn control(&self) -> MutexGuard<'_, Control> {
        self.control.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Main thread only. The control lock is never held across `install` / `remove_all`.
    fn reconcile(&self) {
        let (wanted, installed) = {
            let mut c = self.control();
            c.posted = false;
            (c.wanted && !c.sealed, c.installed)
        };
        // The set the current trust allows; a trust change since the last install differs.
        let desired = wanted.then(|| MonitorSet::for_trust(self.backend.trusted()));
        if desired == installed {
            return;
        }
        if installed.is_some() {
            self.backend.remove_all();
            self.control().installed = None;
        }
        let Some(set) = desired else {
            return;
        };
        if !set.global_key {
            platform_log("escape dismissal unavailable: accessibility not trusted");
        }
        match self.backend.install(set, self.sink.clone()) {
            Ok(()) => self.control().installed = Some(set),
            Err(error) => {
                self.backend.remove_all();
                platform_log(&format!("dismiss monitors install failed: {error}"));
            }
        }
    }

    /// Main thread only: the sealed teardown. Removes every monitor whatever is queued; a
    /// reconcile job that runs later finds the watcher sealed and installs nothing.
    fn remove_sealed(&self) {
        self.backend.remove_all();
        let mut c = self.control();
        c.installed = None;
        c.cleaned = true;
    }
}

/// `DismissWatcher` over a [`MonitorBackend`] (production: `AppKitMonitors`).
pub struct MacDismissWatcher<B: MonitorBackend> {
    inner: Arc<Inner<B>>,
}

impl<B: MonitorBackend> MacDismissWatcher<B> {
    pub fn new(backend: Arc<B>) -> Self {
        Self::with_overlay_active(backend, OverlayActive::new())
    }

    /// `overlay` is the flag the region overlay engine sets while a pick is active
    /// (`native()` shares one between both).
    pub fn with_overlay_active(backend: Arc<B>, overlay: OverlayActive) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(8);
        Self {
            inner: Arc::new(Inner {
                backend,
                sink: Arc::new(DismissSink {
                    active: AtomicBool::new(false),
                    overlay,
                    rect: Mutex::new(Rect::new(0, 0, 0, 0)),
                    sender,
                }),
                receiver,
                main: OnceLock::new(),
                control: Mutex::new(Control::default()),
            }),
        }
    }

    /// Posts one reconcile job unless one is already queued (inline on the main thread).
    fn schedule(&self) -> Result<(), PlatformError> {
        let main = self.inner.main.get().ok_or(PlatformError::Unsupported(
            "dismiss watcher without main thread",
        ))?;
        {
            let mut c = self.inner.control();
            if c.posted {
                return Ok(());
            }
            c.posted = true;
        }
        let inner = self.inner.clone();
        let posted = post_to_main(main.as_ref(), Box::new(move || inner.reconcile()));
        if posted.is_err() {
            self.inner.control().posted = false;
        }
        posted
    }
}

impl<B: MonitorBackend> DismissWatcher for MacDismissWatcher<B> {
    fn start(&self, popup_rect: Rect) -> Result<(), PlatformError> {
        if self.inner.main.get().is_none() {
            return Err(PlatformError::Unsupported(
                "dismiss watcher without main thread",
            ));
        }
        {
            let mut c = self.inner.control();
            if c.sealed {
                return Err(PlatformError::Unsupported("dismiss watcher after shutdown"));
            }
            c.wanted = true;
        }
        *self.inner.sink.rect() = popup_rect;
        while self.inner.receiver.try_recv().is_ok() {}
        self.inner.sink.active.store(true, Ordering::SeqCst);
        self.schedule()
    }

    fn update_rect(&self, popup_rect: Rect) {
        *self.inner.sink.rect() = popup_rect;
    }

    fn stop(&self) {
        self.inner.sink.active.store(false, Ordering::SeqCst);
        self.inner.control().wanted = false;
        let _ = self.schedule();
    }

    fn events(&self) -> crossbeam_channel::Receiver<DismissEvent> {
        self.inner.receiver.clone()
    }

    fn bind_main_thread(&self, main: Arc<dyn MainThread>) {
        let _ = self.inner.main.set(main);
    }

    /// From the INSTALLED set while monitors are installed (a trust granted later only counts
    /// after the next start reinstalls them); with nothing installed, what the next
    /// installation would include.
    fn escape_available(&self) -> bool {
        let installed = self.inner.control().installed;
        match installed {
            Some(set) => set.global_key,
            None => self.inner.backend.trusted(),
        }
    }

    /// Seals the watcher and removes the native monitors before returning: inline on the
    /// main thread (even with a reconcile queued), otherwise through a posted job this thread
    /// waits for, bounded by [`SHUTDOWN_CLEANUP_TIMEOUT`]. Call it holding no lock.
    fn shutdown(&self) {
        self.inner.sink.active.store(false, Ordering::SeqCst);
        {
            let mut c = self.inner.control();
            c.sealed = true;
            c.wanted = false;
            if c.cleaned {
                return;
            }
        }
        // Unbound: `start` never succeeded, so nothing was ever installed.
        let Some(main) = self.inner.main.get() else {
            return;
        };
        if main.is_main_thread() {
            self.inner.remove_sealed();
            return;
        }
        let (done, finished) = crossbeam_channel::bounded::<()>(1);
        let inner = self.inner.clone();
        let job = Box::new(move || {
            inner.remove_sealed();
            let _ = done.send(());
        });
        if main.post(job).is_err() {
            platform_log("dismiss monitors cleanup could not be posted");
            return;
        }
        match finished.recv_timeout(SHUTDOWN_CLEANUP_TIMEOUT) {
            Ok(()) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                platform_log("dismiss monitors cleanup dropped by the main loop");
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                platform_log("dismiss monitors cleanup timed out");
            }
        }
    }
}
