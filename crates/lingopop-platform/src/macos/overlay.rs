//! Portable half of the macOS region overlay, compiled on every OS as
//! `lingopop_platform::macos_overlay` so the whole lifecycle runs on fakes.
//!
//! One `pick` (called on a worker thread, never on the main thread) does:
//! 1. serialize: a second pick waits until the first one's native cleanup is acknowledged, so
//!    at most one native overlay exists at a time;
//! 2. snapshot the cached topology and ask the main thread (waiting OFF it, holding no lock)
//!    to create the native overlay through an [`OverlayBackend`];
//! 3. wait for a result, a cancel (a message or the dropped sender) or a topology change;
//! 4. ask the main thread to remove the overlay AND hand focus back to the previously
//!    frontmost app (`OverlayBackend::restore_previous_app`), wait for that acknowledgment
//!    (bounded), then give the compositor [`COMPOSITOR_SETTLE`] (150 ms, Swift parity), so the capture that follows can never
//!    contain the tint. A panic in between still posts the cleanup (`CleanupGuard`).
//!
//! Every pick takes its own [`OverlayLease`] from the shared [`OverlayActive`] (before the native
//! overlay is created) and releases it only after ITS cleanup ran, on every exit path; a
//! timed-out cleanup that runs late therefore never clears the lease of a newer pick. The
//! macOS dismiss watcher ignores all events while any lease is held, so the overlay's own drag-start click cannot close the
//! popup that was open and abort the flow, whatever order AppKit runs local monitors in.
//!
//! Native input reaches [`OverlaySink::push`] in arrival order. The sink queues it and posts at
//! most ONE drain job to the main thread, so a backlog of drag events is one batch (a
//! `MainThread::post` that runs inline, like Tauri's on the main thread in production, drains
//! each event at once, so batching only happens where the post is really deferred): the shared
//! reducer [`fold_input_batch_with`] folds it in order (Down resets the anchor, Up and Escape
//! end it) and the backend draws only the final selection. The acceptance rule of a finishing
//! `Up` is ten BACKING pixels on the display where the drag started
//! ([`MacGeometry::selection_accepted`]); the result is in desktop (Quartz point) units.
//!
//! A change of the display topology revision while the overlay is up, or before a finished
//! drag is returned, cancels the pick
//! (`Ok(None)`, fixed log line) instead of capturing a region of another layout.
use crate::error::platform_log;
use crate::macos_dismiss::{OverlayActive, OverlayLease};
use crate::macos_geometry::{MacGeometry, TopologyCache};
use crate::main_thread::{dispatch_error, run_on_main, MainThread};
use crate::overlay::{fold_input_batch_with, InputBatch, OverlayInput};
use crate::{PlatformError, Point, Rect, RegionOverlay, Sleeper};
use crossbeam_channel::{bounded, select, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// How often the waiting worker compares the topology revision (no event announces a display
/// change to it: the cache is updated by the main-queue observer).
pub const TOPOLOGY_POLL: Duration = Duration::from_millis(50);
/// Time given to the window server and to the re-activated previous app after the overlay is
/// ordered out, before the capture (Swift `ScreenTranslateFlow` waits 150 ms after
/// re-activating `previousApp`).
pub const COMPOSITOR_SETTLE: Duration = Duration::from_millis(150);
/// Upper bound for the main thread to acknowledge the removal of the overlay.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// The cached topology the overlay reads (shared with `MacMonitors`).
pub trait TopologySource: Send + Sync {
    fn geometry(&self) -> MacGeometry;
    fn topology_revision(&self) -> u64;
}
impl TopologySource for TopologyCache {
    fn geometry(&self) -> MacGeometry {
        self.snapshot()
    }
    fn topology_revision(&self) -> u64 {
        TopologyCache::topology_revision(self)
    }
}

/// The native overlay. Every method is called on the main thread only.
pub trait OverlayBackend: Send + Sync + 'static {
    /// Creates one overlay window per display of `geometry`, installs the input monitors (each
    /// forwarding to `sink` in desktop points, see [`OverlaySink::push`]), shows the windows,
    /// activates the app and sets the crosshair. On `Err` the engine calls `cleanup`.
    fn create(&self, geometry: &MacGeometry, sink: Arc<OverlaySink>) -> Result<(), PlatformError>;
    /// Shows `selection` (desktop points) on every window; `None` = dim only.
    fn draw(&self, selection: Option<Rect>);
    /// Removes monitors and windows and restores the cursor, before returning; idempotent.
    fn cleanup(&self);
    /// Hands focus back to the application that was frontmost when `create` activated ours
    /// (remembered in `create` BEFORE activating; never our own app, only while it still
    /// runs). Called right after `cleanup`, before the engine acknowledges it; idempotent.
    fn restore_previous_app(&self);
}

/// Native teardown of one overlay: windows and monitors first, then focus back to the app
/// the user came from (Swift `ScreenTranslateFlow` re-activates `previousApp`), and only then
/// this pick's lease is released (the dismiss watcher is let through again once no pick holds
/// one; an older, late teardown only ever releases its own pick's lease).
fn teardown(backend: &dyn OverlayBackend, lease: &OverlayLease) {
    backend.cleanup();
    backend.restore_previous_app();
    lease.release();
}

/// The part of the selection that falls on a display, in display-local top-left coordinates
/// (what every overlay window draws). `None` when it does not touch the display or has no area.
pub fn selection_in_display(display: Rect, selection: Rect) -> Option<Rect> {
    let r = display.intersect(selection)?;
    Some(Rect::new(
        r.x.saturating_sub(display.x),
        r.y.saturating_sub(display.y),
        r.width,
        r.height,
    ))
}

#[derive(Default)]
struct Queue {
    events: Vec<OverlayInput>,
    drain_posted: bool,
    active: bool,
}

/// Consumes one drained batch on the main thread.
trait Handler: Send + Sync {
    fn handle(&self, batch: Vec<OverlayInput>);
    fn dispatch_failed(&self);
}

/// Where native input lands. Created by the engine per pick, handed to `create`, and dropped
/// by the backend at cleanup.
pub struct OverlaySink {
    queue: Mutex<Queue>,
    main: Arc<dyn MainThread>,
    handler: OnceLock<Arc<dyn Handler>>,
}

impl OverlaySink {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Called from the native monitor callbacks (main thread) with desktop-point input.
    /// Returns whether the overlay consumes the event: true while the overlay is up, so
    /// the clicks and Escape meant for it never reach other windows.
    pub fn push(self: &Arc<Self>, input: OverlayInput) -> bool {
        let post = {
            let mut q = self.queue();
            if !q.active {
                return false;
            }
            q.events.push(input);
            !std::mem::replace(&mut q.drain_posted, true)
        };
        if post {
            let sink = self.clone();
            // Events that arrive before a deferred drain job runs form one batch.
            if self.main.post(Box::new(move || sink.drain())).is_err() {
                self.queue().drain_posted = false;
                if let Some(handler) = self.handler.get() {
                    handler.dispatch_failed();
                }
            }
        }
        true
    }

    fn drain(&self) {
        let batch = {
            let mut q = self.queue();
            q.drain_posted = false;
            if !q.active {
                return;
            }
            std::mem::take(&mut q.events)
        };
        if let (false, Some(handler)) = (batch.is_empty(), self.handler.get()) {
            handler.handle(batch);
        }
    }

    fn deactivate(&self) {
        let mut q = self.queue();
        q.active = false;
        q.events.clear();
    }
}

type Outcome = Result<Option<Rect>, PlatformError>;

#[derive(Default)]
struct Fold {
    start: Option<Point>,
    rendered: Option<Rect>,
    start_display: Option<u32>,
    finished: bool,
}

enum Step {
    Draw(Rect),
    Finish(Option<Rect>),
}

/// Folds batches on the main thread and reports the result to the waiting worker.
struct Session {
    backend: Arc<dyn OverlayBackend>,
    geometry: MacGeometry,
    fold: Mutex<Fold>,
    done: Sender<Outcome>,
}

impl Handler for Session {
    fn handle(&self, batch: Vec<OverlayInput>) {
        let step = {
            let mut fold = self.fold.lock().unwrap_or_else(|p| p.into_inner());
            if fold.finished {
                return;
            }
            // The acceptance of an `Up` depends on the display of the Down that anchors it.
            for event in &batch {
                match event {
                    OverlayInput::Down(p) => fold.start_display = self.geometry.display_at(*p),
                    OverlayInput::Up(_) | OverlayInput::Escape => break,
                    OverlayInput::Move(_) => {}
                }
            }
            let (geometry, start_display) = (&self.geometry, fold.start_display);
            let accepted = |r: Rect| {
                start_display
                    .and_then(|id| geometry.selection_accepted(id, r).ok())
                    .unwrap_or(false)
            };
            let Fold {
                start, rendered, ..
            } = &mut *fold;
            let folded = fold_input_batch_with(start, *rendered, batch, accepted);
            match folded {
                InputBatch::Unchanged => return,
                InputBatch::Redraw(r) => {
                    fold.rendered = Some(r);
                    Step::Draw(r)
                }
                InputBatch::Finish(result) => {
                    fold.finished = true;
                    Step::Finish(result)
                }
            }
        };
        match step {
            Step::Draw(selection) => self.backend.draw(Some(selection)),
            Step::Finish(result) => {
                let _ = self.done.try_send(Ok(result));
            }
        }
    }

    fn dispatch_failed(&self) {
        let _ = self.done.try_send(Err(dispatch_error()));
    }
}

/// Posts the cleanup (without waiting) if the pick unwinds before it ran.
struct CleanupGuard {
    main: Arc<dyn MainThread>,
    backend: Arc<dyn OverlayBackend>,
    sink: Arc<OverlaySink>,
    lease: OverlayLease,
    armed: bool,
}
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.sink.deactivate();
        let backend = self.backend.clone();
        let lease = self.lease.clone();
        if self
            .main
            .post(Box::new(move || teardown(&*backend, &lease)))
            .is_err()
        {
            // The main loop is gone: no native overlay can outlive it, nothing to wait for.
            platform_log("region overlay cleanup could not be posted");
            self.lease.release();
        }
    }
}

/// `RegionOverlay` over an [`OverlayBackend`] (production: `AppKitOverlay`).
pub struct MacOverlay<B: OverlayBackend> {
    backend: Arc<B>,
    topology: Arc<dyn TopologySource>,
    sleeper: Arc<dyn Sleeper>,
    main: OnceLock<Arc<dyn MainThread>>,
    /// Per-pick leases for the life of each native overlay; read by the dismiss watcher.
    overlay_active: OverlayActive,
    /// Held for a whole pick, cleanup included: the serialization of native overlays.
    serial: Mutex<()>,
    poll: Duration,
}

fn cancelled(cancel: &Receiver<()>) -> bool {
    !matches!(cancel.try_recv(), Err(TryRecvError::Empty))
}

impl<B: OverlayBackend> MacOverlay<B> {
    pub fn new(
        backend: Arc<B>,
        topology: Arc<dyn TopologySource>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Self {
        Self {
            backend,
            topology,
            sleeper,
            main: OnceLock::new(),
            overlay_active: OverlayActive::new(),
            serial: Mutex::new(()),
            poll: TOPOLOGY_POLL,
        }
    }

    /// Shares the flag the dismiss watcher reads (`native()` wires one into both).
    pub fn with_overlay_active(mut self, active: OverlayActive) -> Self {
        self.overlay_active = active;
        self
    }

    /// Tests shorten the topology poll; production uses [`TOPOLOGY_POLL`].
    pub fn with_topology_poll(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    /// Removes the native overlay on the main thread and waits for that (bounded), then lets
    /// the compositor settle. Disarms the guard: the work is done here.
    fn finalize(&self, guard: &mut CleanupGuard) -> Result<(), PlatformError> {
        guard.armed = false;
        guard.sink.deactivate();
        let (ack, acked) = bounded::<()>(1);
        let backend = self.backend.clone();
        let lease = guard.lease.clone();
        if let Err(error) = guard.main.post(Box::new(move || {
            teardown(&*backend, &lease);
            let _ = ack.send(());
        })) {
            // The job never ran and never will (the loop is gone): nothing is left to wait for.
            guard.lease.release();
            return Err(error);
        }
        match acked.recv_timeout(CLEANUP_TIMEOUT) {
            Ok(()) => {
                self.sleeper.sleep(COMPOSITOR_SETTLE);
                Ok(())
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                platform_log("region overlay cleanup dropped by the main loop");
                // Dropped unrun: no native overlay exists any more.
                guard.lease.release();
                Err(dispatch_error())
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // The lease stays held: the queued cleanup releases it when the main thread gets
                // to it, and until then the overlay may still be on screen. A later pick holds
                // its own lease, so this late teardown cannot clear that one.
                platform_log("region overlay cleanup timed out");
                Err(PlatformError::Os {
                    api: "region overlay cleanup",
                    code: 0,
                })
            }
        }
    }
}

enum Wait {
    Cancelled,
    TopologyChanged,
    Done(Outcome),
}

impl<B: OverlayBackend> RegionOverlay for MacOverlay<B> {
    fn pick(&self, cancel: Receiver<()>) -> Outcome {
        let main = self.main.get().cloned().ok_or(PlatformError::Unsupported(
            "region overlay without main thread",
        ))?;
        if main.is_main_thread() {
            // Waiting for the main thread on the main thread would deadlock the very work; and
            // locking `serial` first would block the main thread behind a running pick.
            return Err(PlatformError::Unsupported(
                "region overlay on the main thread",
            ));
        }
        let _serial = self.serial.lock().unwrap_or_else(|p| p.into_inner());
        if cancelled(&cancel) {
            return Ok(None);
        }
        let geometry = self.topology.geometry();
        if geometry.displays().is_empty() {
            platform_log("region overlay: no displays");
            return Err(PlatformError::InvalidRegion);
        }
        let revision = geometry.topology_revision();

        let (done, result) = bounded::<Outcome>(1);
        let session = Arc::new(Session {
            backend: self.backend.clone(),
            geometry: geometry.clone(),
            fold: Mutex::new(Fold::default()),
            done,
        });
        let sink = Arc::new(OverlaySink {
            queue: Mutex::new(Queue {
                active: true,
                ..Queue::default()
            }),
            main: main.clone(),
            handler: OnceLock::new(),
        });
        let _ = sink.handler.set(session);
        let lease = self.overlay_active.begin();
        let mut guard = CleanupGuard {
            main: main.clone(),
            backend: self.backend.clone(),
            sink: sink.clone(),
            lease: lease.clone(),
            armed: true,
        };
        // Before the native overlay exists; released by this pick's `teardown` (or below when
        // no teardown job will ever run).

        let created = run_on_main(main.as_ref(), {
            let backend = self.backend.clone();
            let sink = sink.clone();
            let lease = lease.clone();
            move || {
                let created = backend.create(&geometry, sink);
                if created.is_err() {
                    teardown(&*backend, &lease);
                }
                created
            }
        });
        match created {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                // The create job already ran the cleanup on the main thread.
                guard.armed = false;
                sink.deactivate();
                return Err(error);
            }
            Err(error) => {
                // The job never ran (the loop is gone): nothing exists to clean up.
                guard.armed = false;
                lease.release();
                return Err(error);
            }
        }

        let ticker = crossbeam_channel::tick(self.poll);
        let waited = loop {
            select! {
                recv(cancel) -> _ => break Wait::Cancelled,
                recv(result) -> message => {
                    break Wait::Done(message.unwrap_or_else(|_| Err(dispatch_error())));
                }
                recv(ticker) -> _ => {
                    if self.topology.topology_revision() != revision {
                        break Wait::TopologyChanged;
                    }
                }
            }
        };
        let finished = self.finalize(&mut guard);
        let outcome = match waited {
            Wait::Cancelled => Ok(None),
            Wait::TopologyChanged => {
                platform_log("region overlay cancelled: display topology changed");
                Ok(None)
            }
            // A cancel that raced a finished selection wins: the caller abandoned the pick.
            Wait::Done(_) if cancelled(&cancel) => Ok(None),
            // The display observer and the mouse-up both run on main: a topology published
            // before the Up leaves a rect in OLD desktop coordinates, never hand it out.
            Wait::Done(Ok(Some(_))) if self.topology.topology_revision() != revision => {
                platform_log("region overlay cancelled: display topology changed");
                Ok(None)
            }
            Wait::Done(outcome) => outcome,
        };
        // Without a confirmed cleanup the capture could contain the tint: fail instead.
        finished.and(outcome)
    }

    fn bind_main_thread(&self, main: Arc<dyn MainThread>) {
        let _ = self.main.set(main);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_in_display_clips_and_translates() {
        let d = Rect::new(100, 50, 200, 100);
        assert_eq!(
            selection_in_display(d, Rect::new(150, 60, 400, 20)),
            Some(Rect::new(50, 10, 150, 20))
        );
        assert_eq!(selection_in_display(d, Rect::new(300, 50, 10, 10)), None);
    }
}
