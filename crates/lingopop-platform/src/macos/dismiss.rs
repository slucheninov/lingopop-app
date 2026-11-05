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
//! Every `DismissWatcher` method is non-blocking (the orchestrator calls it under its session
//! mutex): geometry and the active flag are published at once and installation / removal is
//! posted to the main thread as one idempotent reconcile job. Monitor callbacks only filter and
//! `try_send` into a bounded channel.
use crate::error::platform_log;
use crate::main_thread::{post_to_main, MainThread};
use crate::{DismissEvent, DismissWatcher, PlatformError, Point, Rect};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// `kCGEventSourceUserData` value stamped on every CGEvent `MacKeys` posts ("LPOP"), so our
/// own synthesized Cmd+C / Cmd+V never dismiss the popup. Foreign injected input still counts.
pub const OWN_EVENT_MARKER: i64 = 0x4C50_4F50;
/// `kVK_Escape`.
pub const ESCAPE_KEY_CODE: u16 = 53;

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
    rect: Mutex<Rect>,
    sender: crossbeam_channel::Sender<DismissEvent>,
}

impl DismissSink {
    fn rect(&self) -> MutexGuard<'_, Rect> {
        self.rect.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Called from monitor callbacks (main thread). Never blocks beyond the rect read.
    pub fn observe(&self, event: ObservedEvent) {
        if !self.active.load(Ordering::SeqCst) {
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
    /// Monitors are installed (only the main-thread reconcile job changes it).
    installed: bool,
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
        let (desired, installed) = {
            let mut c = self.control();
            c.posted = false;
            (c.wanted && !c.sealed, c.installed)
        };
        if desired == installed {
            return;
        }
        let now_installed = if desired {
            let trusted = self.backend.trusted();
            if !trusted {
                platform_log("escape dismissal unavailable: accessibility not trusted");
            }
            match self
                .backend
                .install(MonitorSet::for_trust(trusted), self.sink.clone())
            {
                Ok(()) => true,
                Err(error) => {
                    self.backend.remove_all();
                    platform_log(&format!("dismiss monitors install failed: {error}"));
                    false
                }
            }
        } else {
            self.backend.remove_all();
            false
        };
        self.control().installed = now_installed;
    }
}

/// `DismissWatcher` over a [`MonitorBackend`] (production: `AppKitMonitors`).
pub struct MacDismissWatcher<B: MonitorBackend> {
    inner: Arc<Inner<B>>,
}

impl<B: MonitorBackend> MacDismissWatcher<B> {
    pub fn new(backend: Arc<B>) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(8);
        Self {
            inner: Arc::new(Inner {
                backend,
                sink: Arc::new(DismissSink {
                    active: AtomicBool::new(false),
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

    fn escape_available(&self) -> bool {
        self.inner.backend.trusted()
    }

    fn shutdown(&self) {
        self.inner.sink.active.store(false, Ordering::SeqCst);
        {
            let mut c = self.inner.control();
            c.sealed = true;
            c.wanted = false;
        }
        let _ = self.schedule();
    }
}
