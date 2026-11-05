//! `ShellPopup`: the orchestrator's `PopupView` over the hidden `popup` Tauri window.
//!
//! Commands are consumed on one dedicated worker ("lingopop-presentation"), never the main
//! thread. The native handle is cached during setup. The worker's [`PresentationConsumer`]
//! emits the model to the webview and runs the window effect through a [`PopupSurface`]:
//! Windows calls `NoActivateWindow` directly from the worker (cross-thread `SetWindowPos`);
//! macOS ([`MacPanelSurface`]) translates the effect into a platform `MacWindowCommand` and
//! runs it on the main thread through `MacPresenter`, waiting for native completion on the
//! worker, which holds no lock while it waits. The staleness check is repeated on the main
//! thread right before AppKit is touched, so a Show superseded by a dismissal never appears.
//! There is no Tauri `show()` / `hide()` fallback on either OS (they may activate the app).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use lingopop_orchestrator::{presentation::*, PopupView, PopupViewModel};
use lingopop_platform::macos_presentation::{
    CurrentCheck, MacPresenter, MacWindowCommand, MacWindowOutcome,
};
use lingopop_platform::{
    monitor_at, physical_size, DismissWatcher, MainThread, Monitors, NoActivateWindow, Platform,
    PlatformError, Point, Rect, WindowHandle,
};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, WebviewWindow};

pub const POPUP_LABEL: &str = "popup";
pub const MODEL_EVENT: &str = "popup://model";

/// Delivers one model JSON to the popup webview.
pub type EmitFn = dyn Fn(&serde_json::Value) + Send + Sync;

/// The newest model, kept so `ready` can re-emit one that arrived before the webview listened.
struct Pending {
    last_model: Option<serde_json::Value>,
}

/// Where the native window effect runs. `Ok(true)` = applied, `Ok(false)` = the command was
/// no longer current when the effect was about to run, so nothing native happened.
pub trait PopupSurface: Send + Sync {
    fn place(&self, rect: Rect, current: &CurrentCheck) -> Result<bool, PlatformError>;
    fn hide(&self, current: &CurrentCheck) -> Result<bool, PlatformError>;
}

fn missing_handle() -> PlatformError {
    log::warn!("popup window handle is unavailable");
    PlatformError::Unsupported("popup window")
}

/// Windows: `NoActivateWindow` straight from the worker (the fence was checked just before).
pub struct PlatformSurface {
    window: Arc<dyn NoActivateWindow>,
    handle: Option<WindowHandle>,
}
impl PlatformSurface {
    pub fn new(window: Arc<dyn NoActivateWindow>, handle: Option<WindowHandle>) -> Self {
        Self { window, handle }
    }
}
impl PopupSurface for PlatformSurface {
    fn place(&self, rect: Rect, _: &CurrentCheck) -> Result<bool, PlatformError> {
        let handle = self.handle.ok_or_else(missing_handle)?;
        self.window.show_at(handle, rect).map(|()| true)
    }
    fn hide(&self, _: &CurrentCheck) -> Result<bool, PlatformError> {
        let handle = self.handle.ok_or_else(missing_handle)?;
        self.window.hide(handle).map(|()| true)
    }
}

/// macOS: the effect runs on the main thread (AppKit) and the worker waits for it.
pub struct MacPanelSurface {
    presenter: MacPresenter,
    handle: Option<WindowHandle>,
}
impl MacPanelSurface {
    pub fn new(
        main: Arc<dyn MainThread>,
        window: Arc<dyn NoActivateWindow>,
        handle: Option<WindowHandle>,
    ) -> Self {
        Self {
            presenter: MacPresenter::new(main, window),
            handle,
        }
    }
    fn run(
        &self,
        command: MacWindowCommand,
        current: &CurrentCheck,
    ) -> Result<bool, PlatformError> {
        self.presenter
            .execute(command, current.clone())
            .map(|outcome| outcome == MacWindowOutcome::Applied)
    }
}
impl PopupSurface for MacPanelSurface {
    fn place(&self, rect: Rect, current: &CurrentCheck) -> Result<bool, PlatformError> {
        let window = self.handle.ok_or_else(missing_handle)?;
        self.run(MacWindowCommand::Show { window, rect }, current)
    }
    fn hide(&self, current: &CurrentCheck) -> Result<bool, PlatformError> {
        let window = self.handle.ok_or_else(missing_handle)?;
        self.run(MacWindowCommand::Hide { window }, current)
    }
}

/// The worker-side consumer: model emission, geometry conversion and the native effect.
pub struct PresentationConsumer {
    emit: Arc<EmitFn>,
    monitors: Arc<dyn Monitors>,
    surface: Box<dyn PopupSurface>,
    /// Asked whether Escape can dismiss (macOS without Accessibility: no) and shut down with
    /// the worker so no native monitor outlives it.
    dismiss: Option<Arc<dyn DismissWatcher>>,
    visible: AtomicBool,
    pending: Mutex<Pending>,
}

/// Reads the native handle. On Windows this blocks on the main thread: call it only from
/// `setup` (main thread), never from `PopupView` methods.
#[cfg(windows)]
pub fn native_handle(window: &WebviewWindow) -> Option<WindowHandle> {
    window
        .hwnd()
        .ok()
        .map(|hwnd| WindowHandle(hwnd.0 as usize as u64))
}

/// macOS: converts the tao window into the non-activating panel (main thread, setup only),
/// through the window's AppKit `HasWindowHandle`. A refused conversion leaves no handle, so
/// every later Show is acknowledged as an error and the popup never appears as an ordinary
/// (activating) window. After the conversion no tao API that relies on `TaoWindow` (e.g.
/// `set_focusable`) may be called on the popup window (see `macos/popup_window.rs`).
#[cfg(target_os = "macos")]
pub fn native_handle(window: &WebviewWindow) -> Option<WindowHandle> {
    match lingopop_platform::configure_popup_panel(window) {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::error!("popup panel configuration failed: {e}");
            None
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn native_handle(_window: &WebviewWindow) -> Option<WindowHandle> {
    None
}

impl PresentationConsumer {
    pub fn new(
        emit: Arc<EmitFn>,
        monitors: Arc<dyn Monitors>,
        surface: Box<dyn PopupSurface>,
    ) -> Self {
        Self {
            emit,
            monitors,
            surface,
            dismiss: None,
            visible: AtomicBool::new(false),
            pending: Mutex::new(Pending { last_model: None }),
        }
    }

    pub fn with_dismiss(mut self, dismiss: Arc<dyn DismissWatcher>) -> Self {
        self.dismiss = Some(dismiss);
        self
    }

    /// The production consumer over the shell's Tauri app and platform bundle.
    pub fn for_app(app: AppHandle, platform: &Platform, handle: Option<WindowHandle>) -> Self {
        let emitter = app.clone();
        let emit: Arc<EmitFn> = Arc::new(move |value: &serde_json::Value| {
            if emitter.emit_to(POPUP_LABEL, MODEL_EVENT, value).is_err() {
                log::warn!("popup model event failed");
            }
        });
        #[cfg(target_os = "macos")]
        let surface: Box<dyn PopupSurface> = Box::new(MacPanelSurface::new(
            Arc::new(crate::macos_main_thread::TauriMacMainThread(app)),
            platform.popup_window.clone(),
            handle,
        ));
        #[cfg(not(target_os = "macos"))]
        let surface: Box<dyn PopupSurface> = {
            drop(app);
            Box::new(PlatformSurface::new(platform.popup_window.clone(), handle))
        };
        Self::new(emit, platform.monitors.clone(), surface).with_dismiss(platform.dismiss.clone())
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn rect_for(&self, origin: Point, logical: (f64, f64)) -> Rect {
        let monitors = self.monitors.monitors().unwrap_or_default();
        let scale = monitor_at(origin, &monitors).map_or(1.0, |m| m.scale_factor);
        let size = physical_size(logical.0, logical.1, scale);
        Rect::new(origin.x, origin.y, size.width, size.height)
    }

    /// Emits `value`, adding the Escape-unavailable notice when the dismiss watcher cannot
    /// observe Escape and the model carries no notice of its own. The cache keeps the plain
    /// model, so a later trust change is reflected on the next emission.
    fn emit_model(&self, value: &serde_json::Value) {
        let escape_missing = self
            .dismiss
            .as_ref()
            .is_some_and(|dismiss| !dismiss.escape_available());
        if escape_missing && value.get("notice").is_none() {
            let mut decorated = value.clone();
            if let Some(object) = decorated.as_object_mut() {
                object.insert(
                    "notice".into(),
                    serde_json::json!({
                        "key": lingopop_orchestrator::messages::KEY_ESCAPE_UNAVAILABLE,
                        "args": [],
                    }),
                );
            }
            (self.emit)(&decorated);
        } else {
            (self.emit)(value);
        }
    }

    /// Shows or moves the window to `rect` without activating it.
    fn place(&self, rect: Rect, current: &CurrentCheck) -> Result<(), PlatformError> {
        match self.surface.place(rect, current) {
            Ok(true) => {
                self.visible.store(true, Ordering::Release);
                Ok(())
            }
            // Stale on the main thread: nothing native ran, visibility is unchanged.
            Ok(false) => Ok(()),
            Err(e) => {
                log::warn!("popup show failed: {e}");
                self.visible.store(false, Ordering::Release);
                Err(e)
            }
        }
    }

    /// The frontend mounted and listens: re-emits a model that arrived before that.
    ///
    /// The worker serializes ready with model updates; no cache lock crosses emit.
    pub fn ready(&self) {
        let model = self.pending().last_model.clone();
        if let Some(model) = model {
            self.emit_model(&model);
        }
    }

    fn show_at(
        &self,
        origin: Point,
        size_logical: (f64, f64),
        model: &PopupViewModel,
        current: &CurrentCheck,
    ) -> Result<(), PlatformError> {
        if let Ok(value) = serde_json::to_value(model) {
            self.pending().last_model = Some(value.clone());
            self.emit_model(&value);
        }
        self.place(self.rect_for(origin, size_logical), current)
    }

    fn update(&self, model: &PopupViewModel) {
        let Ok(value) = serde_json::to_value(model) else {
            return;
        };
        self.pending().last_model = Some(value.clone());
        self.emit_model(&value);
    }

    fn hide(&self, current: &CurrentCheck) {
        let _ = self.hide_checked(current);
    }
    fn hide_checked(&self, current: &CurrentCheck) -> Result<(), PlatformError> {
        if self.surface.hide(current)? {
            self.visible.store(false, Ordering::Release);
        }
        Ok(())
    }

    fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Acquire)
    }

    /// Worker shutdown: no native dismiss monitor may outlive the presentation worker.
    fn shutdown_native(&self) {
        if let Some(dismiss) = &self.dismiss {
            dismiss.shutdown();
        }
    }
}

enum WorkerMessage {
    Drain,
    Ready,
    Shutdown(tokio::sync::oneshot::Sender<()>),
}
/// Queue owner. Dropping it requests worker shutdown without waiting on the UI thread.
pub struct ShellPopup {
    native: Arc<PresentationConsumer>,
    queue: Arc<PresentationQueue>,
    sender: std::sync::mpsc::Sender<WorkerMessage>,
    receiver: Mutex<Option<std::sync::mpsc::Receiver<WorkerMessage>>>,
    sink: OnceLock<Arc<AckSink>>,
}
impl ShellPopup {
    pub fn new(app: AppHandle, platform: Platform, handle: Option<WindowHandle>) -> Self {
        Self::from_consumer(PresentationConsumer::for_app(app, &platform, handle))
    }
    pub fn from_consumer(consumer: PresentationConsumer) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self {
            native: Arc::new(consumer),
            queue: Arc::default(),
            sender,
            receiver: Mutex::new(Some(receiver)),
            sink: OnceLock::new(),
        }
    }
    pub fn ready(&self) {
        let _ = self.sender.send(WorkerMessage::Ready);
    }
    /// Explicit close rejects further submissions immediately. Completion means the worker
    /// has finished its active command and resolved all pending acknowledgments.
    pub fn shutdown(&self) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.queue.seal();
        let _ = self.sender.send(WorkerMessage::Shutdown(tx));
        rx
    }
}
impl Drop for ShellPopup {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}
impl PopupView for ShellPopup {
    fn submit(&self, cmd: PresentationCommand) -> Result<AckReceiver, PresentationQueueError> {
        self.queue
            .submit_notified(cmd, || self.sender.send(WorkerMessage::Drain).is_ok())
    }

    fn bind(&self, fence: Arc<PresentationFence>, sink: Arc<AckSink>) {
        if self.sink.set(sink.clone()).is_err() {
            log::warn!("presentation worker already bound");
            return;
        }
        let receiver = match self.receiver.lock() {
            Ok(mut receiver) => receiver.take(),
            Err(_) => {
                log::warn!("presentation worker receiver lock failed");
                self.queue.close(sink.as_ref());
                return;
            }
        };
        let Some(receiver) = receiver else {
            log::warn!("presentation worker receiver unavailable");
            self.queue.close(sink.as_ref());
            return;
        };
        let queue = self.queue.clone();
        let native = self.native.clone();
        let failed_start_sink = sink.clone();
        let start = std::thread::Builder::new()
            .name("lingopop-presentation".into())
            .spawn(move || {
                let mut desired = DesiredPresentation::default();
                while let Ok(message) = receiver.recv() {
                    match message {
                        WorkerMessage::Ready => native.ready(),
                        WorkerMessage::Shutdown(done) => {
                            native.shutdown_native();
                            queue.close(sink.as_ref());
                            let _ = done.send(());
                            return;
                        }
                        WorkerMessage::Drain => {}
                    }
                    queue.complete_superseded(sink.as_ref());
                    while let Some(job) = queue.try_next() {
                        // Retain desired geometry/content even when a pending Show was fenced
                        // before execution; its current Resize must still be able to show it.
                        desired.record(&job.command);
                        let current: CurrentCheck = {
                            let fence = fence.clone();
                            let id = job.command.id;
                            Arc::new(move || fence.is_current(id))
                        };
                        consume_job(&fence, job, sink.as_ref(), |command| {
                            match &command.kind {
                                PresentationKind::Show {
                                    origin,
                                    size_logical,
                                    model,
                                } => {
                                    if let Err(error) =
                                        native.show_at(*origin, *size_logical, model, &current)
                                    {
                                        native.hide(&current);
                                        return Err(error);
                                    }
                                }
                                PresentationKind::Update { model } => native.update(model),
                                PresentationKind::Resize {
                                    origin,
                                    size_logical,
                                } => {
                                    let placed = desired.resize_with(|_, _, model| {
                                        if native.is_visible() {
                                            native.place(
                                                native.rect_for(*origin, *size_logical),
                                                &current,
                                            )
                                        } else {
                                            native.show_at(*origin, *size_logical, model, &current)
                                        }
                                    });
                                    if let Err(error) = placed {
                                        native.hide(&current);
                                        return Err(error);
                                    }
                                }
                                PresentationKind::Hide => {
                                    native.hide_checked(&current)?;
                                }
                            }
                            Ok(if native.is_visible() {
                                PresentationVisibility::Visible
                            } else {
                                PresentationVisibility::Hidden
                            })
                        });
                        queue.complete_superseded(sink.as_ref());
                    }
                }
                queue.close(sink.as_ref());
            });
        finish_worker_start(start, self.queue.as_ref(), failed_start_sink.as_ref());
    }
}

fn finish_worker_start(
    start: std::io::Result<std::thread::JoinHandle<()>>,
    queue: &PresentationQueue,
    sink: &AckSink,
) {
    if start.is_err() {
        log::warn!("presentation worker failed to start");
        queue.close(sink);
    }
}

#[derive(Default)]
struct DesiredPresentation {
    sequence: u64,
    show: Option<(Point, (f64, f64), PopupViewModel)>,
}
impl DesiredPresentation {
    fn record(&mut self, command: &PresentationCommand) {
        if command.id.sequence <= self.sequence {
            return;
        }
        self.sequence = command.id.sequence;
        match &command.kind {
            PresentationKind::Show {
                origin,
                size_logical,
                model,
            } => self.show = Some((*origin, *size_logical, model.clone())),
            PresentationKind::Update { model } => {
                if let Some((_, _, current)) = self.show.as_mut() {
                    *current = model.clone();
                }
            }
            PresentationKind::Resize {
                origin,
                size_logical,
            } => {
                if let Some((p, size, _)) = self.show.as_mut() {
                    *p = *origin;
                    *size = *size_logical;
                }
            }
            PresentationKind::Hide => self.show = None,
        }
    }
    fn resize_with(
        &self,
        effect: impl FnOnce(
            Point,
            (f64, f64),
            &PopupViewModel,
        ) -> Result<(), lingopop_platform::PlatformError>,
    ) -> Result<(), lingopop_platform::PlatformError> {
        if let Some((origin, size, model)) = &self.show {
            effect(*origin, *size, model)
        } else {
            Ok(())
        }
    }
}

fn consume_job(
    fence: &PresentationFence,
    job: PresentationJob,
    sink: &AckSink,
    effect: impl FnOnce(
        &PresentationCommand,
    ) -> Result<PresentationVisibility, lingopop_platform::PlatformError>,
) {
    let ack = fence
        .apply_if_current(job.command.id, || effect(&job.command))
        .unwrap_or(PresentationAck {
            id: job.command.id,
            result: Ok(PresentationVisibility::Superseded),
            effect_left_visible: false,
        });
    job.complete(ack, sink);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_effect_and_ack_run_off_submitter_and_sink_precedes_oneshot() {
        let queue = Arc::new(PresentationQueue::default());
        let fence = Arc::new(PresentationFence::default());
        let id = PresentationId {
            generation: 1,
            sequence: 1,
        };
        fence.advance(id);
        let submitter = std::thread::current().id();
        let receiver = queue
            .submit(PresentationCommand {
                id,
                kind: PresentationKind::Hide,
            })
            .unwrap();
        let acked = Arc::new(AtomicBool::new(false));
        let seen = acked.clone();
        let (release, blocked) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            blocked.recv().unwrap();
            let job = queue.try_next().unwrap();
            consume_job(
                &fence,
                job,
                &move |_| {
                    seen.store(true, Ordering::SeqCst);
                },
                |_| {
                    assert_ne!(std::thread::current().id(), submitter);
                    Ok(PresentationVisibility::Hidden)
                },
            );
            queue.close(&|_| {});
        });
        assert!(!acked.load(Ordering::SeqCst));
        release.send(()).unwrap();
        assert_eq!(
            receiver.await.unwrap().result,
            Ok(PresentationVisibility::Hidden)
        );
        assert!(acked.load(Ordering::SeqCst));
        worker.join().unwrap();
    }
}

#[cfg(test)]
mod fix_round_one_tests {
    use super::*;
    #[test]
    fn hide_clears_resize_intent_so_no_native_effect_runs() {
        let mut desired = DesiredPresentation::default();
        let id = |sequence| PresentationId {
            generation: 1,
            sequence,
        };
        desired.record(&PresentationCommand {
            id: id(1),
            kind: PresentationKind::Show {
                origin: Point { x: 0, y: 0 },
                size_logical: (400., 200.),
                model: PopupViewModel::loading(
                    lingopop_core::operation::AIOperation::Translate,
                    "fixture",
                ),
            },
        });
        desired.record(&PresentationCommand {
            id: id(2),
            kind: PresentationKind::Hide,
        });
        desired.record(&PresentationCommand {
            id: id(3),
            kind: PresentationKind::Resize {
                origin: Point { x: 10, y: 10 },
                size_logical: (500., 300.),
            },
        });
        let mut effects = 0;
        desired
            .resize_with(|_, _, _| {
                effects += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(effects, 0);
    }
    #[tokio::test]
    async fn worker_start_failure_closes_and_resolves_accepted_work() {
        let queue = PresentationQueue::default();
        let id = PresentationId {
            generation: 1,
            sequence: 1,
        };
        let receiver = queue
            .submit(PresentationCommand {
                id,
                kind: PresentationKind::Hide,
            })
            .unwrap();
        finish_worker_start(Err(std::io::Error::other("fixture")), &queue, &|_| {});
        assert_eq!(
            receiver.await.unwrap().result,
            Ok(PresentationVisibility::Superseded)
        );
        assert_eq!(
            queue
                .submit(PresentationCommand {
                    id,
                    kind: PresentationKind::Hide
                })
                .unwrap_err(),
            PresentationQueueError::Closed
        );
    }
}

#[cfg(test)]
#[path = "popup_shell_mac_tests.rs"]
mod mac_consumer_tests;
